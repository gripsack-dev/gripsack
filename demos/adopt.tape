# gripsack CLI demo — adopt an existing config: the ownership question,
# scoped take-over with priors, then a rollback that RESTORES the files.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-adopt.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@40ms "sh /vhs/demos/adopt_setup.sh && cd /tmp && grip init myenv && cd myenv"
Enter
Sleep 6s
Type@40ms "cat ~/.config/helix/config.toml"
Enter
Sleep 3s
Type@40ms "grip adopt ~/.config/helix"
Enter
Sleep 5s
Up
Sleep 2s
Enter
Sleep 3s
Type@40ms "y"
Enter
Sleep 6s
Type@40ms "grip why-owns ~/.config/helix/config.toml"
Enter
Sleep 4s
Type@40ms "grip rollback"
Enter
Sleep 4s
Type@40ms "cat ~/.config/helix/config.toml"
Enter
Sleep 4s
