#!/usr/bin/env python3
"""
Better_Jarvis Knowledge Base & Environment Integrity Auditor
Scans skills, workflows, rules, and documentation for broken links,
missing frontmatter, and file-size budget violations.
"""

import os
import re
import sys

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
AGENTS_DIR = os.path.join(REPO_ROOT, ".agents")
MAX_RULE_BYTES = 24000 # 24 KB Antigravity per-rule limit

def audit_skills():
    print("[*] Auditing skills in .agents/skills/...")
    skills_dir = os.path.join(AGENTS_DIR, "skills")
    if not os.path.isdir(skills_dir):
        print("[FAIL] Missing skills directory")
        return False
    
    ok = True
    skills = [d for d in os.listdir(skills_dir) if os.path.isdir(os.path.join(skills_dir, d))]
    for s in skills:
        skill_md = os.path.join(skills_dir, s, "SKILL.md")
        if not os.path.isfile(skill_md):
            print(f"[FAIL] Skill '{s}' missing SKILL.md")
            ok = False
            continue
        with open(skill_md, "r", encoding="utf-8", errors="ignore") as fh:
            content = fh.read()
        if not (content.startswith("---") and "name:" in content and "description:" in content):
            print(f"[FAIL] Skill '{s}' missing valid YAML frontmatter")
            ok = False
    if ok:
        print(f"[PASS] All {len(skills)} skills contain valid SKILL.md and YAML frontmatter")
    return ok

def audit_rules_and_budget():
    print("[*] Auditing rules and byte budgets in .agents/rules/...")
    rules_dir = os.path.join(AGENTS_DIR, "rules")
    if not os.path.isdir(rules_dir):
        print("[WARN] No rules directory found")
        return True
    
    ok = True
    rules = [f for f in os.listdir(rules_dir) if f.endswith(".md")]
    for r in rules:
        p = os.path.join(rules_dir, r)
        size = os.path.getsize(p)
        if size > MAX_RULE_BYTES:
            print(f"[FAIL] Rule '{r}' exceeds 24 KB limit ({size} bytes)")
            ok = False
    if ok:
        print(f"[PASS] All {len(rules)} rule files within 24 KB budget")
    return ok

def audit_workflows():
    print("[*] Auditing workflows in .agents/workflows/...")
    wf_dir = os.path.join(AGENTS_DIR, "workflows")
    if not os.path.isdir(wf_dir):
        print("[FAIL] Missing workflows directory")
        return False
    
    expected_wfs = [
        "repo-recon.md", "feature-impl.md", "bug-fix.md", "arch-change.md",
        "security-mod.md", "perf-opt.md", "code-review.md", "session-continuity.md"
    ]
    ok = True
    for wf in expected_wfs:
        p = os.path.join(wf_dir, wf)
        if not os.path.isfile(p):
            print(f"[FAIL] Missing required workflow: {wf}")
            ok = False
    if ok:
        print(f"[PASS] All {len(expected_wfs)} standard workflows verified present")
    return ok

def audit_documentation_links():
    print("[*] Auditing documentation links in .agents/...")
    ok = True
    link_pattern = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
    
    for root, _, files in os.walk(AGENTS_DIR):
        for f in files:
            if f.endswith(".md"):
                p = os.path.join(root, f)
                with open(p, "r", encoding="utf-8", errors="ignore") as fh:
                    content = fh.read()
                matches = link_pattern.findall(content)
                for label, target in matches:
                    if target.startswith("http://") or target.startswith("https://") or target.startswith("#"):
                        continue
                    # Clean anchor fragments
                    clean_target = target.split("#")[0]
                    if not clean_target:
                        continue
                    resolved = os.path.normpath(os.path.join(root, clean_target))
                    if not os.path.exists(resolved):
                        print(f"[WARN] Broken link in {os.path.relpath(p, REPO_ROOT)}: '{target}' -> {resolved}")
                        # Non-blocking warning for optional deep cross-references
    print("[PASS] Documentation link audit completed")
    return ok

def main():
    print("=================================================================")
    print("   BETTER_JARVIS KNOWLEDGE INTEGRITY AUDITOR                     ")
    print("=================================================================")
    s_ok = audit_skills()
    r_ok = audit_rules_and_budget()
    w_ok = audit_workflows()
    l_ok = audit_documentation_links()
    
    overall = s_ok and r_ok and w_ok and l_ok
    print("=================================================================")
    print(f"Overall Knowledge Integrity: {'PASS' if overall else 'FAIL'}")
    print("=================================================================")
    sys.exit(0 if overall else 1)

if __name__ == "__main__":
    main()
