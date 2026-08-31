# gripsack CLI demo — real apply flow against a fixture env repo.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@40ms "# a fixture env repo: one tarball tool, one dotfiles module"
Enter
Sleep 3s
Type@40ms "sh /vhs/demos/demo_setup.sh && cd /tmp/myenv"
Enter
Sleep 4s
Type@40ms "# apply - fetch, deploy, one new generation"
Enter
Sleep 2s
Type@40ms "grip apply --host demo"
Enter
Sleep 4s
Type@40ms "# run the deployed binary"
Enter
Sleep 2s
Type@40ms "~/.local/bin/hello"
Enter
Sleep 4s
Type@40ms "# a second apply is a satisfied no-op"
Enter
Sleep 2s
Type@40ms "grip apply --host demo"
Enter
Sleep 4s
Type@40ms "# the flip history"
Enter
Sleep 2s
Type@40ms "grip generations"
Enter
Sleep 4s
Type@40ms "# plan validates and shows execution waves - errors point at your code"
Enter
Sleep 2s
Type@40ms "grip plan --ir /vhs/demos/fixtures/ir.json"
Enter
Sleep 4s
