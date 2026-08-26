# gripsack CLI demo — grip check: a linter error with a rendered
# snippet, fixed, then green. Zero side effects either way.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-check.gif
Set FontSize 14
Set Width 900
Set Height 560
Set Theme "Catppuccin Mocha"

Type@10ms "sh /vhs/demos/check_setup.sh && cd /tmp/myenv"
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
