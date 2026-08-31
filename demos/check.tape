# gripsack CLI demo — grip check: a linter error with a rendered
# snippet, fixed, then green. Zero side effects either way.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-check.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@40ms "# a repo with a typo'd config key"
Enter
Sleep 3s
Type@40ms "sh /vhs/demos/check_setup.sh && cd /tmp/checkenv"
Enter
Sleep 4s
Type@40ms "# check catches it before anything stages"
Enter
Sleep 2s
Type@40ms "grip check --host demo"
Enter
Sleep 4s
Type@40ms "# fix the typo"
Enter
Sleep 2s
Type@40ms "sed -i 's/BAD_KEY/auto_save/' configs/demo/demo.toml"
Enter
Sleep 4s
Type@40ms "# green - the CI gate for your dotfiles repo"
Enter
Sleep 2s
Type@40ms "grip check --host demo"
Enter
Sleep 4s
