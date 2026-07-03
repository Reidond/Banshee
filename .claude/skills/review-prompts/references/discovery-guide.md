# Prompt Discovery Guide

This reference helps the reviewer locate and understand prompt architecture in any project.

## How to Discover Prompts

### 1. File-Based Prompts
Search for prompt template files:
```
find . -name "*.txt" -path "*/prompt*" -type f
find . -name "*.md" -path "*/prompt*" -type f
find . -name "*.jinja2" -o -name "*.j2" | grep -i prompt
```

### 2. Embedded Prompts
Search for prompts embedded in source code:
```
grep -rn "system.*prompt\|You are a\|Your role is" --include="*.py" --include="*.ts"
```

### 3. Configuration-Driven Prompts
Search for prompt configuration:
```
grep -rn "prompt_template\|system_message\|prompt_path" --include="*.py" --include="*.yaml"
```

## Common Prompt Architecture Patterns

### Pattern A: Template Files + Builder
```
prompts/              ← Template files (.txt, .md)
  └── feature/
      ├── system.txt  ← System prompt template
      └── user.txt    ← User prompt template with {placeholders}

backend/app/
  ├── core/prompts/   ← Base loader, builder, sanitizer
  └── features/{f}/
      └── services/
          ├── prompt_loader.py   ← Loads templates
          ├── prompt_builder.py  ← Assembles final prompt
          └── models/ai_requests/ ← Pydantic models for template variables
```

### Pattern B: Inline Prompts in Service Code
```
backend/app/features/{f}/services/
  └── ai_service.py   ← Prompts defined as string constants or f-strings
```

### Pattern C: Configuration-Driven
```
config/prompts.yaml   ← Prompts defined in config
backend/app/core/
  └── prompt_manager.py ← Loads from config
```

## What to Map for Each Prompt

For every prompt file discovered, identify:

1. **Template variables**: `{placeholder}` patterns in the template
2. **Parameter source**: Which model/class supplies the variable values
3. **Assembly pipeline**: How the template becomes a final prompt
   - Fragment injection (`_fragments/`)
   - Config value injection (settings)
   - Language/locale injection
   - User data sanitization and wrapping
4. **AI provider target**: Which model/API receives this prompt
5. **Feature owner**: Which feature/module uses this prompt
6. **Variant axis**: What creates different versions (seniority, difficulty, type, etc.)

## Grouping Strategies

When reviewing 50+ prompts, group by:

| Strategy | When to Use |
|----------|------------|
| By feature | Default — review each feature's prompts together |
| By function | When prompts across features serve the same purpose (all scoring prompts) |
| By variant axis | When reviewing calibration (all junior prompts, all hard prompts) |
| By shared component | Review _shared/ and _fragments/ first, then consumers |

## Review Ordering

Recommended order for maximum efficiency:

1. **Shared components first** (`_shared/`, `_fragments/`) — issues here cascade
2. **System prompts before user prompts** — system defines the rules
3. **Core/main prompts before variants** — establish the baseline
4. **Simple prompts before complex** — build understanding progressively
