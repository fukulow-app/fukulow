---
applyTo: ".github/workflows/**"
---

# Workflow review

The rules are in the continuous-integration section of
`docs/architecture/invariants.md`.

**Each item below is a condition to flag.** Comment when the workflow meets it.

- **Flag** a workflow with no `permissions:` block, or with permissions broader
  than it uses. Without the block it inherits the repository default, which can be
  read-write and can change without the file changing.
- **Flag** an action referenced by tag or branch rather than a **full commit SHA**
  with its version in a comment. A tag can be moved to different code.
- **Flag** an `actions/checkout` without `persist-credentials: false`. The token
  otherwise stays on disk for every later step.
- **Flag** a tool installed at `latest` or with no version, where the workflow
  exists to check pinning.
- **Flag** `pull_request_target` combined with a checkout of the pull request's
  head. That runs untrusted code with a privileged token.
- **Flag** an attacker-controlled value — `${{ github.event.pull_request.title }}`,
  a branch name, a comment body — interpolated directly into a `run:` script. Pass
  it through an environment variable.
- **Flag** a `name:` added to, or a rename of, the `build`, `audit` or
  `signed-off-by` jobs. Those names are the status checks `main` requires.
- **Flag** a job whose failure could mean two unrelated things — for example the
  code being wrong and a new advisory being published. Each failure should mean
  one thing.
