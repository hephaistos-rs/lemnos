# Lemnos
**Forge your own cloud.**

## Running it

Run one Lemnos process per database. Running several against the same database
is not supported yet: each process keeps track of the machines it is creating by
itself, so two of them could work on the same machine at once.
