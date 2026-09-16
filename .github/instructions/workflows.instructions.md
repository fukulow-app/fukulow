---
applyTo: ".github/workflows/**"
---

# Workflow review

- Does the workflow declare `permissions:`, and are they the least it needs? A
  workflow without them inherits the repository default, which can be read-write
  and can change without the file changing.
- Is every third-party action pinned to a **full commit SHA**, with the version in a
  comment? A tag can be moved to different code.
- Is `pull_request_target` used together with a checkout of the pull request's
  head? That runs untrusted code with a privileged token.
- Is an expression such as `${{ github.event.pull_request.title }}` or any other
  attacker-controlled value interpolated directly into a `run:` script? Pass it
  through an environment variable instead.
- Does a job that can fail for a reason unrelated to the change — an advisory, an
  external service — sit in the same job as one that means the code is wrong?
  Each job's failure should mean one thing.
