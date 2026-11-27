"""Which paths `quality.yml` gates, by its `paths` filter, for the scripts that need it.

`xtask/tests/full_mutation_workflows.rs` checks the filter is still the one this mirrors.
"""


def gated(path):
    """Whether a push changing `path` runs quality.yml: everything but Markdown and
    LICENSE, and the protocol's Markdown again."""
    if path.startswith("docs/for-humans/protocol/") and path.endswith(".md"):
        return True
    return not (path.endswith(".md") or path == "LICENSE")
