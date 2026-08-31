# gripsack CLI demo — rollback: two applies, one flip back.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-rollback.gif
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
Type@40ms "sed -i 's/catppuccin-mocha/nord/' editor.toml && grip apply --host demo"
Enter
Sleep 4s
Type@40ms "cat ~/.config/editor/editor.toml"
Enter
Sleep 4s
Type@40ms "grip rollback"
Enter
Sleep 4s
Type@40ms "cat ~/.config/editor/editor.toml"
Enter
Sleep 4s
Type@40ms "grip generations"
Enter
Sleep 4s
