#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    Fast,
    Heavy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelection {
    pub tier: ModelTier,
    pub model_name: String,
    pub reason: &'static str,
}

pub struct ModelRouter {
    pub fast_model: String,
    pub heavy_model: String,
}

impl ModelRouter {
    pub const PROMPT_LENGTH_THRESHOLD: usize = 240;

    pub fn new(fast_model: String, heavy_model: String) -> Self {
        Self {
            fast_model,
            heavy_model,
        }
    }

    /// Pure, deterministic model tier selector.
    pub fn select_tier(
        &self,
        utterance: &str,
        has_active_taint: bool,
        prior_tool_turns: usize,
    ) -> ModelSelection {
        // 1. Dynamic Escalation: Tainted context requires heavy model for hardened prompt-injection resistance
        if has_active_taint {
            return ModelSelection {
                tier: ModelTier::Heavy,
                model_name: self.heavy_model.clone(),
                reason: "dynamic_escalation_tainted_context",
            };
        }

        // 2. Dynamic Escalation: Multi-turn ReAct tool loops require heavy model
        if prior_tool_turns >= 1 {
            return ModelSelection {
                tier: ModelTier::Heavy,
                model_name: self.heavy_model.clone(),
                reason: "dynamic_escalation_multiturn_react",
            };
        }

        let trimmed = utterance.trim();

        // 3. Prompt length threshold
        if trimmed.len() > Self::PROMPT_LENGTH_THRESHOLD {
            return ModelSelection {
                tier: ModelTier::Heavy,
                model_name: self.heavy_model.clone(),
                reason: "prompt_length_exceeds_threshold",
            };
        }

        let lower = trimmed.to_lowercase();

        // 4. Multi-clause chained instructions
        if lower.contains(" and then ")
            || lower.contains(" and after that ")
            || lower.contains(" then ")
            || (lower.contains("compare ") && lower.contains(" and "))
        {
            return ModelSelection {
                tier: ModelTier::Heavy,
                model_name: self.heavy_model.clone(),
                reason: "multi_clause_chained_instructions",
            };
        }

        // 5. Code, debug, planning, refactor, architecture analysis intent
        let complex_keywords = [
            "debug",
            "refactor",
            "architecture",
            "plan ",
            "planning",
            "analyze",
            "implement",
            "write code",
            "code for",
            "explain code",
            "review code",
            "find bug",
            "solve",
            "algorithm",
        ];

        for kw in complex_keywords {
            if lower.contains(kw) {
                return ModelSelection {
                    tier: ModelTier::Heavy,
                    model_name: self.heavy_model.clone(),
                    reason: "complex_reasoning_or_code_keywords",
                };
            }
        }

        // 6. Browser extraction / multi-source research intent
        let research_keywords = [
            "extract",
            "scrape",
            "research",
            "browse and summarize",
            "search and compare",
            "investigate",
        ];

        for kw in research_keywords {
            if lower.contains(kw) {
                return ModelSelection {
                    tier: ModelTier::Heavy,
                    model_name: self.heavy_model.clone(),
                    reason: "browser_extraction_or_research",
                };
            }
        }

        // Default to Fast model for short conversational Q&A, definitions, single-fact lookups
        ModelSelection {
            tier: ModelTier::Fast,
            model_name: self.fast_model.clone(),
            reason: "fast_conversational_default",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_router() -> ModelRouter {
        ModelRouter::new(
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-pro".to_string(),
        )
    }

    #[test]
    fn test_fast_tier_conversational_qa() {
        let router = test_router();
        let sel = router.select_tier("What time is it?", false, 0);
        assert_eq!(sel.tier, ModelTier::Fast);
        assert_eq!(sel.model_name, "gemini-2.5-flash");
        assert_eq!(sel.reason, "fast_conversational_default");

        let sel2 = router.select_tier("Remember that my name is Youssef", false, 0);
        assert_eq!(sel2.tier, ModelTier::Fast);
    }

    #[test]
    fn test_heavy_tier_complex_code_and_debug() {
        let router = test_router();
        let sel = router.select_tier("Please debug this ring buffer memory ordering issue", false, 0);
        assert_eq!(sel.tier, ModelTier::Heavy);
        assert_eq!(sel.model_name, "gemini-2.5-pro");
        assert_eq!(sel.reason, "complex_reasoning_or_code_keywords");

        let sel2 = router.select_tier("Refactor the audio capture pipeline", false, 0);
        assert_eq!(sel2.tier, ModelTier::Heavy);
        assert_eq!(sel2.reason, "complex_reasoning_or_code_keywords");
    }

    #[test]
    fn test_heavy_tier_long_prompt() {
        let router = test_router();
        let long_prompt = "a".repeat(241);
        let sel = router.select_tier(&long_prompt, false, 0);
        assert_eq!(sel.tier, ModelTier::Heavy);
        assert_eq!(sel.reason, "prompt_length_exceeds_threshold");
    }

    #[test]
    fn test_heavy_tier_multi_clause_chained_instructions() {
        let router = test_router();
        let sel = router.select_tier("Open my documents folder and then check the status file", false, 0);
        assert_eq!(sel.tier, ModelTier::Heavy);
        assert_eq!(sel.reason, "multi_clause_chained_instructions");

        let sel2 = router.select_tier("Compare option A and option B for deployment", false, 0);
        assert_eq!(sel2.tier, ModelTier::Heavy);
        assert_eq!(sel2.reason, "multi_clause_chained_instructions");
    }

    #[test]
    fn test_dynamic_escalation_tainted_context() {
        let router = test_router();
        // Even a trivial 3-word query must escalate if context is tainted
        let sel = router.select_tier("Hello there", true, 0);
        assert_eq!(sel.tier, ModelTier::Heavy);
        assert_eq!(sel.reason, "dynamic_escalation_tainted_context");
    }

    #[test]
    fn test_dynamic_escalation_multiturn_react() {
        let router = test_router();
        // Subsequent tool-calling turns escalate to heavy model
        let sel = router.select_tier("Next step", false, 1);
        assert_eq!(sel.tier, ModelTier::Heavy);
        assert_eq!(sel.reason, "dynamic_escalation_multiturn_react");
    }
}
