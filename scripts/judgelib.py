"""What every judge script asks about a labeled hunk: the state text and the
nouls, in one place so a judge is trained and scored on the same words."""
import os

# the judge is a jeff server (~/repo/jeff); nothing here talks to a hosted API
JEFF_URL = os.environ.get("JEFF_URL", "http://127.0.0.1:8017")

QUESTIONS = {
    "faithful": "Does the rationale accurately describe what this diff changes?",
    "matches_commit": "Is the rationale consistent with the commit message?",
    "noise_correct": "Is this diff only formatting, whitespace, generated code or import lines, with no behavioural change?",
}


def state_text(r):
    return (f"commit message: {r['subject']}\nrationale: {r['rationale']}\n"
            f"diff:\n{r['diff'][:6000]}")


def noul_instructions(r):
    """qid -> instruction text, one per labeled question and one per finding."""
    L = r["labels"]
    qs = {k: v for k, v in QUESTIONS.items() if k in L and L[k] is not None}
    for name in L["findings"]:
        msg = next((f["message"] for f in r["findings"] if f["name"] == name), name)
        qs[f"finding:{name}"] = f"Is this reviewer warning warranted for this code? Warning: {msg}"
    return qs


def noul_labels(r):
    """qid -> bool for every noul `noul_instructions` asks that has a label."""
    L = r["labels"]
    out = {k: L[k] for k in QUESTIONS if k in L and L[k] is not None}
    out.update({f"finding:{n}": lab for n, lab in L["findings"].items() if lab is not None})
    return out
