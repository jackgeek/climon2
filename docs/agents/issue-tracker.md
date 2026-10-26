# Issue tracker: GitHub

Issues and specs live in GitHub Issues. Use the `gh` CLI for all operations. Infer the repository from `git remote -v`; `gh` does this automatically inside a clone.

## Conventions

- Create: `gh issue create --title "..." --body "..."`. Use a heredoc for multiline bodies.
- Read: `gh issue view <number> --json number,title,body,labels,comments`.
- List: `gh issue list --state open --json number,title,body,labels,comments`, with appropriate `--label` and `--state` filters.
- Comment: `gh issue comment <number> --body "..."`.
- Apply or remove labels: `gh issue edit <number> --add-label "..."` or `--remove-label "..."`.
- Close: `gh issue close <number> --comment "..."`.
- Sub-issues: `gh issue create --parent <parent> ...` or `gh issue edit <parent> --add-sub-issue <child>` (`gh` 2.94+). Older clients: `gh api --method POST repos/<owner>/<repo>/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>`. Where sub-issues are unavailable, add the child to a task list in the parent and put `Part of #<parent>` atop the child body.

## Pull requests as a triage surface

**PRs as a request surface: no.** Set to `yes` to include external PRs in triage.

When enabled, use the same labels and states with `gh pr` equivalents:

- Read: `gh pr view <number> --comments` and `gh pr diff <number>`.
- List external PRs: `gh api --paginate 'repos/{owner}/{repo}/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, labels: [.labels[].name]}'`.
- Comment, label, or close: `gh pr comment`, `gh pr edit --add-label` / `--remove-label`, and `gh pr close`.

GitHub shares issue and PR numbers. Resolve ambiguous references with `gh pr view <number>`, falling back to `gh issue view <number>`.

## Skill operations

When a skill says "publish to the issue tracker", create a GitHub issue. When it says "fetch the relevant ticket", read the issue using the command above.

## Wayfinding operations

Used by `/wayfinder`:

- Map: one issue labelled `wayfinder:map`, with Notes / Decisions-so-far / Fog sections.
- Child ticket: a sub-issue of the map, labelled `wayfinder:<type>` (`research`, `prototype`, `grilling`, or `task`). Once claimed, assign it to the driving developer.
- Blocking: use native dependencies: `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`. Obtain the database ID with `gh api repos/<owner>/<repo>/issues/<number> --jq .id`; it is not the issue number or node ID. Where dependencies are unavailable, put `Blocked by: #<number>, #<number>` atop the child body.
- Frontier: list the map's open children, exclude assigned tickets and tickets with open blockers (`issue_dependencies_summary.blocked_by > 0`, or open issues in the fallback line), then select the first in map order.
- Claim: `gh issue edit <number> --add-assignee @me`, the session's first write.
- Resolve: comment with the answer, close the child, then append a summary and link to the map's Decisions-so-far.
