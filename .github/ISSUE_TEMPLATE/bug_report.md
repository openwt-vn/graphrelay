---
name: Bug report
about: Something isn't working correctly
labels: bug
---

## What happened

<!-- Describe the problem clearly. What did you try to do, and what went wrong? -->

## Steps to reproduce

1. 
2. 
3. 

## Expected behavior

<!-- What should have happened? -->

## Actual behavior

<!-- What actually happened? Include error messages verbatim. -->

## Environment

```
OS:              # e.g. macOS 14.5 Apple Silicon / Ubuntu 24.04 x86_64
graphrelay:       # output of: gr --help | head -1
Claude Code:     # output of: claude --version (if applicable)
Project size:    # approx files / LOC
Languages:       # e.g. TypeScript, Python
```

## gr status

```
# paste output of: gr status
```

## Daemon log

```
# paste output of: tail -50 ~/.graphrelay/daemon.log
# For crashes or unexpected behavior, restart with RUST_LOG=debug and reproduce:
#   gr stop && RUST_LOG=debug gr start
```

## Additional context

<!-- Any other context, screenshots, or information that might help. -->
