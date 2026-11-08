# Archive

Superseded material, kept because it is evidence, not because it is current.
Nothing here is maintained, imported, run by CI, or cited as present behavior.
If a fact in here is still true, the living document already states it.

## Contents

| Path | What it is | Why it is here | Current owner |
|---|---|---|---|
| `prd12-superseded-status.md` | The per-version delivery status that PRD 12 carried for 0.0.1 to 0.0.9, verbatim | PRD 12 now records only the current release and the next target, so a superseded statement cannot be read as present status | [PRD 12](../prd/PRD_12_platform_delivery.md), [releases/](../releases/) |
| `probes-0.0.6/` | Four Windows native probe harnesses written for 0.0.6 journeys: create, missing-recent locate, navigation return and editor/terminal handoff | They assert the 0.0.6 window shape and are cited by no plan, PRD or workflow after that version closed. They import shared helpers from `scripts/` and are records, not runnable from here | the maintained probes in [scripts/](../scripts/) |

## Rules for this directory

- Never link here as proof of current behavior. Cite the version plan or the release notes instead.
- Never edit an archived record to match today. Add a note to the living document and leave the record alone.
- Never run an archived script as part of a release gate.
- Deleting from here requires the same justification as deleting evidence anywhere else: the tag or release it describes is gone.
