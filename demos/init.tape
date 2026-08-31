# gripsack CLI demo — first run: grip init scaffolds a working env repo.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-init.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@40ms "# start from nothing - scaffold an env repo"
Enter
Sleep 3s
Type@40ms "rm -rf /tmp/my-env ~/.local/share/gripsack && cd /tmp && grip init my-env && cd my-env"
Enter
Sleep 4s
Type@40ms "# what did we get?"
Enter
Sleep 2s
Type@40ms "find . -type f -not -path './.git/*' | sort"
Enter
Sleep 4s
Type@40ms "# check - eval + sema + linters, zero side effects"
Enter
Sleep 2s
Type@40ms "grip check"
Enter
Sleep 4s
Type@40ms "# first apply deploys the hello module"
Enter
Sleep 2s
Type@40ms "grip apply"
Enter
Sleep 4s
Type@40ms "# the deployed file, linked from the store"
Enter
Sleep 2s
Type@40ms "cat ~/.config/hello/hello.toml"
Enter
Sleep 4s
Type@40ms "# generation 1 is on disk"
Enter
Sleep 2s
Type@40ms "grip generations"
Enter
Sleep 4s
