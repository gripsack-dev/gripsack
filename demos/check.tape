# gripsack CLI demo — grip check: a linter error with a rendered
# snippet, fixed, then green. Zero side effects either way.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-check.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@10ms "sh /vhs/demos/check_setup.sh && cd /tmp/checkenv"
Enter
Sleep 1s
Type@10ms "grip check"
Enter
Sleep 3s
Type@10ms "sed -i 's/BAD_KEY/auto_save/' configs/demo/demo.toml"
Enter
Sleep 1s
Type@10ms "grip check"
Enter
Sleep 3s
