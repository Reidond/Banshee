# Prompt Engineering Review Dimensions

## 1. Structural Clarity

Does the prompt have a clear, logical structure that the LLM can follow?

**Check for:**
- Role definition: Is there a clear persona/role assignment? Is it specific enough?
- Task framing: Is the task stated upfront before details?
- Section separation: Are distinct instruction blocks visually and logically separated?
- Instruction ordering: Are instructions in the order the LLM should process them?
- Hierarchy: Do headings/sections create a clear priority hierarchy?

**Anti-patterns:**
- Instructions buried deep in a wall of text
- Contradictory instructions at different positions (last-position bias wins)
- Critical constraints placed before context (model hasn't loaded context yet)
- Role definition that's too generic ("You are a helpful assistant")

---

## 2. Output Format Specification

Is the expected output format clearly defined and enforceable?

**Check for:**
- Explicit format declaration (JSON, markdown, plain text)
- Schema definition for structured output (field names, types, required vs optional)
- Example output showing exact structure
- Edge case handling (what to output when data is missing/empty)
- Format consistency across related prompts (same feature should use same schema)

**Anti-patterns:**
- "Return a JSON response" without specifying the schema
- Format implied by examples but never explicitly stated
- Multiple possible interpretations of the output structure
- Missing field descriptions for ambiguous field names
- No guidance on array length, string length, or value constraints

---

## 3. Parameter Alignment with AI Request Models

Do the prompt's template variables match the Pydantic model that supplies them?

**Check for:**
- Every `{placeholder}` in the template has a corresponding field in the AI request model
- No unused model fields (indicates dead parameters or missing prompt usage)
- Data types match expectations (e.g., prompt treats a field as a list but model defines it as a string)
- Optional fields: does the prompt handle the case where they're None/empty?
- Enum values: does the prompt reference all possible enum values the model can provide?

**Anti-patterns:**
- Template uses `{experience_level}` but model has `seniority` field
- Model field is Optional but prompt doesn't handle missing value
- Prompt hardcodes values that should come from the model
- Fragment injection overwrites placeholder the model also fills

---

## 4. Instruction Specificity and Actionability

Are instructions specific enough that two different LLM runs would produce similar outputs?

**Check for:**
- Concrete actions vs vague guidance ("list 3 specific improvements" vs "provide feedback")
- Quantified constraints (character limits, number of items, score ranges)
- Decision criteria for conditional behavior (when to do X vs Y)
- Scope boundaries (what NOT to do is as important as what to do)
- Calibration anchors (examples of good vs bad output)

**Anti-patterns:**
- "Be thorough" / "Be concise" without defining what that means
- "Provide feedback" without specifying dimensions, format, or depth
- Conditional behavior without clear trigger conditions
- Instructions that rely on subjective interpretation ("appropriate level of detail")

---

## 5. Token Efficiency

Is the prompt using tokens effectively without unnecessary bloat?

**Check for:**
- Repeated instructions across system and user prompts
- Verbose phrasing that could be shortened without losing meaning
- Redundant examples (one good example > three mediocre ones)
- Copy-pasted sections across prompts that should be shared fragments
- Headers/formatting that consume tokens without adding LLM-useful structure

**Anti-patterns:**
- Same instruction stated 3 different ways "for emphasis"
- Long preambles before the actual task
- Excessive markdown formatting the LLM doesn't need
- Duplicated content between system prompt and user prompt
- Boilerplate text repeated in every prompt of a category

---

## 6. Guard Rails and Edge Cases

Does the prompt handle edge cases and prevent common failure modes?

**Check for:**
- Empty/missing input handling (e.g., what if a transcript is empty? what if an expected document is missing?)
- Boundary conditions (e.g., mismatched input combinations, empty Q&A pairs)
- Injection resistance (user content properly wrapped/sandboxed)
- Hallucination prevention (does it say "only use provided information"?)
- Refusal handling (what should the AI do when it can't fulfill the request?)
- Language consistency (if multilingual, are all instructions language-aware?)

**Anti-patterns:**
- No mention of what to do with empty/null input
- Assumes all input data will be well-formed
- User-provided text concatenated directly into instructions
- "Be creative" without constraining the creativity
- No fallback behavior for ambiguous inputs

---

## 7. Consistency Across Prompt Family

Are related prompts (same feature, different variants) consistent with each other?

**Check for:**
- Same terminology for same concepts across all prompts in a feature
- Consistent output schema fields and naming
- Consistent scoring/rating scales
- Variant prompts (e.g., seniority-specific) that actually differ meaningfully
- Difficulty variants that represent distinct calibration levels

**Anti-patterns:**
- "experience_level" in one prompt, "seniority" in another for the same concept
- Different output field names for the same data across related prompts
- Variant prompts that are copy-pasted with trivial word changes
- Difficulty variants that don't meaningfully change expectations

---

## 8. LLM-Specific Best Practices

Does the prompt follow known best practices for the target LLM (GPT-4, etc.)?

**Check for:**
- Positive instructions ("do X") over negative ("don't do Y") where possible
- Critical instructions at the end of the prompt (recency bias)
- XML/markdown delimiters for multi-section prompts
- Few-shot examples for complex output formats
- Chain-of-thought encouragement for reasoning-heavy tasks
- System prompt vs user prompt role separation (system = identity/rules, user = task/data)

**Anti-patterns:**
- All instructions in system prompt, no structure in user prompt
- Critical constraints only at the beginning (positional neglect)
- Relying on "don't" instructions for important behaviors
- No examples for non-trivial output formats
- Mixing identity instructions with task data
