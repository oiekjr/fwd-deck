# AGENTS.md

## Required Completion Checks

Before completing any task, always run the following commands:

1. `task fmt`
2. `task app:format`
3. `task check`

If any command fails, fix the cause and rerun the required checks until tests, linting, and formatting checks all pass.

## Documentation Guidelines

When adding or changing user-facing behavior, configuration, commands, release behavior, or operational workflow, update the related documentation in the same task. If no documentation update is needed, explicitly confirm that the existing documentation remains accurate.

When a Markdown document intentionally needs a rendered line break inside a paragraph, end the preceding line with two ASCII spaces. Do not rely on source line breaks alone for rendered line breaks.

In Japanese Markdown documents, normally insert a line break immediately after a Japanese full stop (`。`). If that line break is intended as a rendered line break, the preceding line MUST end with two ASCII spaces.

## Conditional Compilation Guidelines

When adding or changing code behind OS-specific `cfg` attributes, apply the same `cfg` boundary to helper functions, constants, enum variants, and tests that exist only for that platform-specific path.
Do not leave platform-specific support code compiled on other operating systems unless it is intentionally shared.

## Japanese Text Spacing

Do not mechanically insert half-width spaces before or after numbers or English words in Japanese comments and documents.

Keep numbers, units, counters, and suffixes together when they form one semantic expression.

- Good: `1つ`, `4種類`, `100万件以内`
- Bad: `1 つ`, `4 種類`, `100 万件以内`

Keep English abbreviations and Japanese nouns together when they form one compound technical term.

- Good: `API仕様書`, `DB接続`, `CSV出力`
- Bad: `API 仕様書`, `DB 接続`, `CSV 出力`

Insert a half-width space only when an English word or abbreviation is syntactically separate from the following Japanese phrase, such as before a particle.

- Good: `API の`, `React で`, `CSV を出力する`
- Bad: `APIの`, `Reactで`, `CSVを出力する`
