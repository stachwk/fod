# FOD versioning

`fod_version.txt` is the authoritative FOD version and stores the version used by the latest commit.

## Commit rule

Every new commit must increment the patch component before it is created:

```text
x.y.z -> x.y.(z + 1)
```

For example, a repository whose latest committed version is `3.2.6` must use `3.2.7` for the next commit.

The same change must:

1. update `fod_version.txt`;
2. align `[workspace.package].version` in `Cargo.toml`;
3. regenerate `Cargo.lock` when Cargo package metadata changes;
4. use the exact commit subject format `FOD X.Y.Z: <english description>`;
5. verify the published runtime version with `make test-version`.

A patch version must not be reused by two different commits. Major or minor version changes require an explicit project decision; otherwise only the patch component is incremented.

If repository history and `fod_version.txt` disagree, first align both version files to one greater than the highest FOD version already used on the current branch. Do not rewrite published history solely to repair numbering.
