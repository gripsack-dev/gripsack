# gripsack CLI demo — first run: grip init scaffolds a working env repo.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-init.gif
Set FontSize 14
Set Width 900
Set Height 560
Set Theme "Catppuccin Mocha"

Type@10ms "cd /tmp && grip init my-env && cd my-env"
Enter
Sleep 2s
Type@10ms "find . -type f -not -path './.git/*' | sort"
Enter
Sleep 1.5s
Type@10ms "grip check"
Enter
Sleep 3s
Type@10ms "grip apply"
Enter
Sleep 3s
Type@10ms "cat ~/.config/hello/hello.toml"
Enter
Sleep 2s
Type@10ms "grip generations"
Enter
Sleep 2s
