# Definition of Ready (DoR) Pre-Flight Tables

Answer each verification question with evidence from the document — "Yes (evidence)", "No", or "Unknown". Any "No" or "Unknown" becomes a finding with the pre-assigned severity.

## Requirements DoR Table

Apply when reviewing `requirements.md` or `spec.md`:

| Verification question | If "No" or "Unknown" | Severity |
|-----------------------|---------------------|----------|
| Does the problem statement explain *why* with measurable impact or user pain? | Completeness — problem rationale missing | Major |
| Are all actors identified (who initiates, who is affected)? | Completeness — actors undefined | Major |
| Do all acceptance scenarios use a valid format — either Gherkin (User Stories with Given/When/Then) OR Use Cases (Main Success Scenario + at least one Exception Flow)? | No verifiable criteria — plan cannot be tested | Blocker |
| Does every acceptance scenario have adversarial coverage — Gherkin error Scenarios OR Use Case Exception Flows? | Missing adversarial coverage | Major |
| Are AI-specific behaviors specified where the feature involves AI model interaction? | Missing AI failure contract | Major |
| Can a test be written for each scenario or use case step independently? | Criteria are not independently verifiable | Major |
| Do NFRs have quantifiable targets (no adjectives: "fast", "properly", "good")? | Vague non-functional requirements | Minor |
| Are scope boundaries explicit — both in-scope deliverables and out-of-scope exclusions? | Scope ambiguity | Minor |
| Are dependencies and constraints identified? | Implicit assumptions | Minor |

## Design/Plan DoR Table

Apply when reviewing `design.md`, `spec.md` (combined), or a Cursor plan:

| Verification question | If "No" or "Unknown" | Severity |
|-----------------------|---------------------|----------|
| Does every Gherkin scenario or Use Case (main flow + alternatives + exceptions) map to at least one implementation task? | Orphan requirements — untraceable to work | Major |
| Does the risk assessment cover AI-specific risks where the feature involves AI? | Missing AI failure modes in risk model | Major |
| Is there a rollback strategy for schema changes or API contract changes? | Missing rollback path | Major (if DB/API touched) |
| Does the testing strategy cover scenario-level or use-case-step verification? | Untestable plan | Major |
| Are the files to create and files to modify explicitly listed? | Implicit scope | Minor |
| Are external AI service integration points documented with their failure modes? | Missing failure contract at AI boundary | Major |
