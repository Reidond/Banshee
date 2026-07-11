# Domain Review Dimensions — Generic Reference

These dimensions apply to any domain. During the domain discovery step, the reviewer
maps these generic dimensions to the specific domain at hand.

## 1. Terminology and Concept Accuracy

Are domain terms used correctly and consistently?

**Check for:**
- Terms match industry-standard definitions
- Concepts are not conflated (e.g., using "assessment" and "evaluation" interchangeably when they mean different things in the domain)
- Acronyms and abbreviations are used correctly
- Process names match the methodology they claim to implement
- Domain hierarchy is preserved (e.g., "dimension" vs "criteria" vs "indicator")

**Red flags:**
- Terms that have been redefined from their standard meaning without explanation
- Inconsistent terminology across related prompts
- Made-up terms when standard terms exist

---

## 2. Methodology Implementation Fidelity

Are referenced methodologies implemented correctly?

**Check for:**
- All components of the methodology are present (not just some)
- Components are in the correct order/relationship
- The methodology is applied to appropriate use cases
- Common misconceptions about the methodology are avoided
- Adaptations from the standard methodology are intentional and documented

**Red flags:**
- Missing components that are critical to the methodology's validity
- Methodology applied to a use case it wasn't designed for
- Known misconceptions embedded in the implementation
- "Franken-methodology" — parts of different methodologies mixed incoherently

---

## 3. Calibration and Scoring Validity

Are assessment criteria properly calibrated?

**Check for:**
- Clear, observable behavioral anchors for each score level
- Meaningful differentiation between adjacent score levels
- Evidence-based scoring (requires citing specific observations)
- Bias mitigation (no penalizing valid alternative approaches)
- Discrimination power (criteria can distinguish between performance levels)
- Inter-rater reliability potential (two AI runs should agree)

**Red flags:**
- Vague criteria ("shows competency") without observable indicators
- Ceiling/floor effects (most inputs would score the same)
- Cultural or stylistic bias baked into criteria
- Overlapping dimensions that measure the same thing twice
- Missing "middle ground" — criteria only describe extremes

---

## 4. Difficulty/Complexity Gradients

Do different difficulty or complexity levels represent meaningful distinctions?

**Check for:**
- Each level has distinct expectations, not just rewording
- Progression follows domain-appropriate complexity scaling
- The hardest level is achievable but challenging for the target audience
- The easiest level is not trivially simple for the target audience
- Transitions between levels reflect how the domain actually works

**Red flags:**
- Levels that differ only in adjective intensity ("good" → "very good" → "excellent")
- Copy-pasted content with trivial word substitutions
- Difficulty that changes only one dimension when multiple should change
- Gaps or overlaps between levels

---

## 5. Role and Persona Authenticity

For simulation prompts: Does the AI persona behave as a real professional would?

**Check for:**
- Role-appropriate scope (doesn't ask questions outside its expertise)
- Stage-appropriate behavior (matches the interaction point in the process)
- Professional boundaries maintained
- Realistic conversation flow and transitions
- Domain-appropriate communication register

**Red flags:**
- Persona acting outside its professional scope
- Unrealistic behavior that breaks immersion
- Missing professional boundaries (asking inappropriate questions)
- Monologue behavior (never acknowledging the user's input)

---

## 6. Pedagogical Soundness (for coaching/teaching prompts)

Does the coaching approach follow effective learning principles?

**Check for:**
- Scaffolding: builds on what the learner already knows
- Specificity: references the learner's actual input, not generic advice
- Actionability: every feedback point includes a "try this" action
- Positive-first: strengths acknowledged before improvements
- Progressive difficulty: adapts as the learner improves
- Growth mindset framing: developmental, not judgmental

**Red flags:**
- Generic feedback applicable to anyone
- All-negative feedback with no strength acknowledgment
- Tells what's wrong without explaining how to fix it
- Doesn't reference the learner's actual work
- Numerical scoring in a learning context (scores inhibit learning)

---

## 7. Edge Case and Boundary Handling

Does the prompt handle domain edge cases?

**Check for:**
- Out-of-scope input (user provides irrelevant content)
- Boundary cases (input at the edge of two categories)
- Missing/empty data (required domain information not provided)
- Contradictory input (conflicting signals in the data)
- Domain-specific failure modes (what goes wrong in this specific domain?)

**Red flags:**
- No mention of edge case handling
- Assumes all input will be well-formed and on-topic
- No redirect strategy for out-of-scope input
- Missing fallback for ambiguous domain data

---

## 8. Ethical and Professional Standards

Does the prompt uphold domain-specific ethical standards?

**Check for:**
- Professional codes of conduct referenced or upheld
- Bias awareness built into assessment and feedback
- Cultural sensitivity in cross-cultural contexts
- Privacy and confidentiality considerations
- Appropriate disclaimers where needed
- Fair treatment regardless of demographic factors

**Red flags:**
- Assessment criteria that penalize cultural communication differences
- Missing bias mitigation in scoring/evaluation
- Ethically sensitive topics handled without guardrails
- Professional standards that apply to this domain being ignored
