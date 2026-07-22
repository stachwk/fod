# Local Test Data Privacy

## Purpose

Local validation may run against real files, mounted shares, device exports, or other data that must not be exposed through GitHub pull requests, issues, comments, commit messages, or release notes.

This document defines the default rule for publishing local FOD test results.

## Default rule

Treat all locally observed file and directory metadata as private unless the repository already contains that exact value as synthetic test data.

Do not publish:

- absolute or home-relative filesystem paths;
- real file or directory names;
- source names derived from a hostname, username, mount point, device, or share;
- usernames, hostnames, volume labels, share names, or device identifiers;
- raw `stdout` or `stderr` from scans, hashes, imports, mounts, or benchmarks when it may contain local paths;
- complete database rows, hashes, timestamps, or identifiers copied from a personal catalogue unless they are required and explicitly approved.

## Pull-request and issue descriptions

Summarize validation with outcomes and aggregate counters only. Prefer statements such as:

```text
31 unit tests passed
integration regression passed
3003 files scanned
76 duplicate sets found
```

Do not paste a complete local log merely to prove that a test passed.

When a failure must be documented, describe the failing phase, assertion, exit status, and safe aggregate values. Replace local values with neutral placeholders or synthetic fixture names.

## Safe data

The following are safe by default:

- repository paths, for example `rust_indexer/src/hash.rs`;
- command names and options;
- test names;
- aggregate counts and durations;
- synthetic fixtures committed to the repository, for example `source-a`, `a.txt`, or `/tmp/fod-indexer-test-*`;
- shortened commit identifiers and software versions.

A synthetic name is safe only when it is created by the test suite and is not copied from the local filesystem.

## Publishing workflow

Before creating or updating a pull request, issue, comment, or release note:

1. Review the proposed text independently from the terminal log.
2. Include only the minimum validation summary needed for review.
3. Search the text for usernames, hostnames, mount roots, home directories, source names, and recognizable document names.
4. Do not attach local logs unless the user explicitly approves the sanitized content.
5. Keep raw logs local. Use them for diagnosis, not as routine GitHub evidence.
6. Run the privacy checker against the prepared text.

For a saved PR body:

```bash
python3 tools/check_github_text_privacy.py /tmp/pr-body.md
```

For stdin:

```bash
printf '%s\n' '31 unit tests passed' |
python3 tools/check_github_text_privacy.py --quiet
```

The checker exits with status `1` when it detects a likely local path, shell identity, raw indexer progress line, current-file field, catalogue hash, or catalogue timestamp. Diagnostics include only a rule name and line number; they do not repeat the detected value.

The checker is a guard, not proof that arbitrary prose is anonymous. A final human review is still required because a recognizable file or directory name may appear without a path.

## Privacy-gated PR creation

Prefer the wrapper when creating a pull request locally:

```bash
python3 tools/create_safe_pr.py \
  --title "Describe the change" \
  --body-file /tmp/pr-body.md \
  --base main \
  --head "$(git branch --show-current)"
```

The wrapper validates both the title and body before running `gh pr create`. It passes the body through `--body-file`; the body content is not copied into a command-line argument or diagnostic message. Pull requests are drafts by default. Use `--ready` only when the change should immediately be ready for review.

Validate without contacting GitHub:

```bash
python3 tools/create_safe_pr.py \
  --title "Describe the change" \
  --body-file /tmp/pr-body.md \
  --dry-run
```

When validation fails, GitHub CLI is not invoked. The wrapper reports only rule names and line numbers and does not repeat detected local values.

## Explicit approval

Local filenames, paths, or logs may be published only when the user explicitly requests that specific content to be included after reviewing it. Approval to run a local test is not approval to publish its output.

## Rationale

FOD tests can legitimately inspect personal catalogues while validating scanning and duplicate detection. The functional result may be useful to the project, but the underlying catalogue metadata is unrelated to the code review and can disclose private information. Separating local diagnostic evidence from the public or shared review summary preserves both test usefulness and data privacy.
