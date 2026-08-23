# gripsack CLI demo — placeholder until `grip apply` lands (plan/0004+).
# The tape exercises what exists so the render pipeline is proven now.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo.gif
Set FontSize 14
Set Width 900
Set Height 560
Set Theme "Catppuccin Mocha"

Type@10ms "grip --version"
Enter
Sleep 1s
Type@10ms "grip doctor"
Enter
Sleep 2s
Type@10ms "grip plan --ir /vhs/demos/fixtures/ir.json"
Enter
Sleep 3s
