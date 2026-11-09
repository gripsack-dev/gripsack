# syntax=docker/dockerfile:1
# d2 renderer for docs/diagrams (plan/0056). Pinned by digest; built with BuildKit.
FROM terrastruct/d2:v0.7.1@sha256:810f69aff66636a75ab0b14afaca7d9055698ebe3a54274b477b351dffef9b58
WORKDIR /diagrams
ENTRYPOINT ["/bin/sh", "-c", "for f in /diagrams/*.d2; do d2 \"$f\" \"${f%.d2}.svg\" || exit 1; done"]
