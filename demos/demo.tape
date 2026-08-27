# gripsack CLI demo — real apply flow against a fixture env repo.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@40ms "sh /vhs/demos/demo_setup.sh && cd /tmp/myenv"
Enter
Sleep 4s
Type@40ms "grip apply --host demo"
Enter
Sleep 4s
Type@40ms "~/.local/bin/hello"
Enter
Sleep 4s
Type@40ms "grip apply --host demo"
Enter
Sleep 4s
Type@40ms "grip generations"
Enter
Sleep 4s
Type@40ms "grip plan --ir /vhs/demos/fixtures/ir.json"
Enter
Sleep 4s
