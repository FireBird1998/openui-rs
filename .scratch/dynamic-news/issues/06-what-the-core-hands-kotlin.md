# 06 — What does the core hand Kotlin on each chunk?

Type: grilling
Status: open
Blocked by: 01 (and openui-lang ticket 07)

## Question

During a streaming parse, does the Kotlin side get the whole parse result after every chunk, or only what changed since the last one? Which shape lets Compose redraw only the changed parts of a story screen, and what does each cost to send across the boundary?
