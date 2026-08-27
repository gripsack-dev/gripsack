# gripsack CLI demo — rollback: two applies, one flip back.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-rollback.gif
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
Type@10ms "sed -i 's/catppuccin-mocha/nord/' editor.toml && grip apply"
Enter
Sleep 3s
Type@10ms "cat ~/.config/editor/editor.toml"
Enter
Sleep 1.5s
Type@10ms "grip rollback"
Enter
Sleep 2s
Type@10ms "cat ~/.config/editor/editor.toml"
Enter
Sleep 2s
Type@10ms "grip generations"
Enter
Sleep 2s
