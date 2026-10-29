# Fixture graphs from Graphify 0.9.86

Copies of `../<lang>.clustered.graphify.json`, four raw `--no-cluster` graphs and
`python.graphify_analysis.json` as Graphify 0.9.86 built them, before the extraction goldens moved
to 0.9.97. They are the input graphs of the query, MCP, export, multi-repo, diagnose, memory and PR
goldens, which record what Graphify's commands print for a given graph; freezing the input keeps
those goldens independent of extraction changes.
