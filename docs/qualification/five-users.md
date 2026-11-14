# Five-participant court (C7)

The gate: **4 of 5 participants complete clone → stage/commit → pull → push
unaided.** This file fixes the protocol before the first participant is
recruited, so the result cannot be redefined after the fact.

## Participants

- Five people who have never used SrcMgr.
- The target user from 01: an individual developer who works across several
  local repositories and knows what a commit and a push are. Someone who has
  never used Git at all is a different study, not this one.
- Record only the role and the Git experience band ("daily", "weekly", "rare").
  No names, no accounts, no contact details.

## Setup

- A machine with the product installed as a user would install it.
- A remote the participant may push to (a throwaway repository per participant).
- A clone URL that is not the product's own repository.
- The observer says one sentence — the task — and nothing else:

  > Clone this repository, change one file, commit the change, pull, and push.

- The observer does not point, does not name keys, and does not answer "how".
  If the participant asks, the answer is: "What would you try next?"

## What counts

A participant **completes unaided** when all five steps finish inside the window
without the observer touching the keyboard or naming a control. Time is recorded
but is not a pass condition.

The court passes when at least four of five complete unaided.

## Record

```text
participant | git band | completed | stopped at | first confusion (their words) | recovered alone
P1          | daily    | yes       | -          | -                            | -
P2          | weekly   | no        | pull       | "it said diverged"           | no
...
```

Also record, for each stop: the exact message the product showed, and whether the
participant found the next action without help. A stop caused by a real defect
is a release blocker; a stop caused by wording or affordance is either fixed or
recorded as a limitation in the release notes.

## Where results live

`plan/evidence/five-users/<version>/results.md`, with screenshots of any message a
participant could not act on. Nothing identifying a person is stored.

## What this court does not prove

- Accessibility or non-pointer operation.
- Any cell other than the one used.
- That five people represent all users; it is a floor, not a survey.
