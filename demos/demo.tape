# gripsack CLI demo — real apply flow against a fixture env repo.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@10ms "sh /vhs/demos/demo_setup.sh && cd /tmp/myenv"
Enter
Sleep 1s
Type@10ms "grip apply"
Enter
Sleep 3s
Type@10ms "~/.local/bin/hello"
Enter
Sleep 1s
Type@10ms "grip apply"
Enter
Sleep 2s
Type@10ms "grip generations"
Enter
Sleep 2s
Type@10ms "grip plan --ir /vhs/demos/fixtures/ir.json"
Enter
Sleep 3s
