# My workspace

`gripsack.ts` declares named outputs. The starter `hello` profile deploys
`configs/hello/hello.toml` to `~/.config/hello/hello.toml` as a store-backed
symlink. Edit the repository file, not the deployed symlink.

```sh
grip trust inspect .
grip trust add .
grip check
grip plan
grip apply
grip generations
grip rollback
```

Approval is bound to captured source bytes and policy. Review and renew it
after source changes. `env.toml` excludes editor-only `node_modules` from
capture; evaluation uses the embedded SDK. Optional `npm install` downloads
the matching SDK archive from the core release for editor types. Linux and
WSL on x86_64 are supported.

To import existing files, run `grip adopt PATH`, review the generated profile
and copied files, approve the changed workspace, then follow the printed
`grip adopt PATH --resume` command. Original destination state is retained
for rollback.

Documentation: https://gripsack.dev
