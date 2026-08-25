# Host entrypoint for this machine — selected by `grip apply --host <name>`
# (default: the machine's hostname, so name the file after it).
#
# Tags drive per-host variation. In a module:
#
#     from gripsack import facts
#     if facts.has("work"):
#         ...  # work-only entries
#
tags = [
    # "work",
    # "laptop",
]
