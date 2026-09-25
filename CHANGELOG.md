# Unreleased

# 0.2.0 - 2026-09-25

- Add mouse/hit-testing support: `into_document*` functions return a
  `MarkdownDocument` pairing the rendered `Text` with per-span element
  annotations. `MarkdownDocument::element_at` maps Text-space coordinates and
  `element_at_screen` maps absolute screen coordinates (widget area, scroll,
  word wrap, and alignment aware) to the Markdown element under the cursor,
  including its ancestor chain. See the new `mouse` example.

# 0.1.0 - 2026-03-16

- initial commit with basic functionality
