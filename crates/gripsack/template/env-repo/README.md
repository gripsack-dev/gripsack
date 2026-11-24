# My workspace

`gripsack.ts` declares named outputs. The starter `hello` profile deploys
`configs/hello/hello.toml` to `~/.config/hello/hello.toml` as a store-backed
symlink. Edit the repository file, not the deployed symlink.

```sh
grip check
grip plan
grip apply
grip generations
grip rollback
```

Running grip on a repository on your disk is consent: there is nothing to
approve. `grip plan` is the review. It highlights new hook bodies, new fetch
domains and destinations outside `~`, and `grip apply` asks before running
them (pass `--yes` in scripts and CI). A repository fetched with
`--repo <git url>` is trusted on first use, once per origin. `env.toml` excludes editor-only `node_modules` from
capture; evaluation uses the embedded SDK. Optional `npm install` downloads
the matching SDK archive from the core release for editor types. Linux and
WSL on x86_64 are supported.

To import existing files, run `grip adopt PATH`, review the generated profile
and copied files, then follow the printed
`grip adopt PATH --resume` command. Original destination state is retained
for rollback.

Documentation: https://gripsack.dev
