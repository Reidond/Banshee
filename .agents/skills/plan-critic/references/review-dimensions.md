# Review Dimensions — Detailed Criteria

## Dimension 1 — Security

Applies when the plan introduces: new endpoints, new auth flows, new service integrations, data writes, or external API calls.

**Auth & Access**
- Is authentication explicitly stated — not just inherited by assumption?
- Are authorization boundaries defined per operation (not just at the entry point)?
- Are new service-to-service calls using JWT auth via `BaseInternalClient`?
- Are internal endpoints excluded from public OpenAPI docs?

**Input & Data**
- Where does untrusted input enter the system? Is it validated before use?
- Is sensitive data masked in logs, error responses, and API payloads?
- Are secrets read from `core/config.py` — not hardcoded or read via `os.getenv()` in services?

**Surface Area**
- Are new public endpoints accompanied by a threat model or at least a note on rate limiting?

> Automatic flag: any plan that defers security to a later phase is incomplete.

---

## Dimension 2 — Overengineering

**Abstraction Justification**
- Does each new interface, protocol, or abstraction layer have a concrete justification beyond "best practice"?
- Is an external library or service being added for functionality achievable in 5–10 lines?

**Async / Event / Distributed**
- Is async, event-driven, or distributed architecture proposed where a simple synchronous call suffices?

**YAGNI**
- Is the plan solving the stated problem — or a generalized future version of it?
- Are "nice to have" features bundled into the core implementation without acknowledgment?

> Challenge trigger: "What breaks if we remove this component?" — if nothing breaks today, justify its presence.

---

## Dimension 3 — Stability & Resilience

**Failure Modes**
- What happens when each external dependency (AI provider, DB, internal service) is unavailable?
- Is retry logic bounded — max attempts, backoff, timeout defined?
- Does the feature degrade gracefully, or does it fail completely?

**Concurrency & Data Integrity**
- Are there race conditions in the proposed flow (e.g., concurrent WebSocket events, parallel task creation)?
- Is idempotency required and designed for where operations may be retried?
- Are DB transactions scoped correctly — not too broad (locking hot rows), not too narrow (partial writes)?

**Observability**
- Are structured logs, metrics, or traces part of the plan for new async or background flows?
- Can an on-call engineer diagnose a production failure from what this plan produces?

> Automatic flag: any external I/O with no failure handling is an incomplete plan.

---

## Dimension 4 — Impact & Affected Areas

**Breaking Changes**
- Does this modify an existing API contract (request shape, response shape, HTTP status, WebSocket event type)?
- Are there consumers of changed interfaces outside the stated scope? (frontend, other backend features, internal clients)
- Does this change the wire format of any WebSocket event? Stale-client behavior must be defined.

**Database & Data Layer**
- Does this add, modify, or remove schema elements? Is there an Alembic migration?
- Is the migration reversible? Will it lock tables under production load (e.g., `ALTER TABLE` on a large table)?
- Are queries going through repositories — not called directly from services?

**Deployment & Rollout**
- Does this require infrastructure changes not captured in the plan (env vars, secrets, Docker, cloud deployment config)?
- Is there a rollback path? (migration downgrade, feature flag, old endpoint preserved?)

**Performance**
- Does this add latency or query overhead to existing hot paths (e.g., WebSocket message handlers, real-time streams)?
- Are performance assumptions backed by evidence?

> Challenge trigger: "What existing behavior changes — even unintentionally?"

---

## Dimension 5 — Plan Completeness

**Definition of Done**
- Is "done" defined in verifiable, specific terms — not just "feature works"?
- Do acceptance criteria use Gherkin Given/When/Then format (for user-facing scenarios) OR Use Case format with Main Success Scenario + Exception Flows (for system/technical flows)?
- Does every Gherkin scenario or Use Case (including alternative and exception flows) map to at least one task?

**Testing Strategy**
- Are unit, integration, and edge case tests included?
- Do all tests use timeout decorators (`fast_test`, `integration_test`, `websocket_test`)?
- Are external services mocked via DI overrides — not patched directly?

**Scope Hygiene**
- Is out-of-scope explicitly stated?
- Are cross-team or cross-feature dependencies confirmed — not assumed?
- Are assumptions listed? Is the plan still valid if any assumption breaks?

**Scenario Quality** *(applies when the document contains Gherkin scenarios)*
- Are scenarios declarative (behavior-focused), not imperative (UI-mechanics)?
- Is `Background` used where 2+ scenarios share preconditions, avoiding repetition?
- Are `Scenario Outlines` used instead of duplicated scenarios with different data?
- Do scenarios cover the error boundary (auth, validation, service unavailable, invalid input)?
- Are AI-specific scenarios present where the feature involves AI model interaction?
- Is terminology consistent across all scenarios (same actor names, same domain terms)?

**Use Case Quality** *(applies when the document contains Use Cases)*
- Does every Use Case have at least one Exception Flow? (happy-path-only Use Cases are incomplete)
- Are Postconditions (Failure) stated, enforcing invariants (no partial writes, error logged)?
- Is the Main Success Scenario ≤10 steps? (longer flows should be split into sub-use-cases)
- Are Alternative Flows clearly distinguished from Exception Flows? (alternatives = valid variants; exceptions = failures)
- Does the primary actor match the Actors table and clearly represent a system component (not a human user)?

> Automatic flag: "We'll figure out the details during implementation" is deferred planning, not a plan.
> Automatic flag: A Feature with only happy-path Gherkin scenarios is incomplete.
> Automatic flag: A Use Case with no Exception Flows is incomplete.
