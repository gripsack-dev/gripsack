# Existing-file adoption uses an explicit profile and reviewed takeover.
Output demos/demo-adopt.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@40ms "# capture existing configuration in a workspace first"
Enter
Sleep 3s
Type@40ms "sh /vhs/demos/adopt_setup.sh && cd /tmp/myenv"
Enter
Sleep 4s
Type@40ms "cat gripsack.ts"
Enter
Sleep 4s
Type@40ms "# inspect the ownership decision before authorizing takeover"
Enter
Sleep 2s
Type@40ms "grip plan editor --take-over"
Enter
Sleep 4s
Type@40ms "y"
Enter
Sleep 4s
Type@40ms "grip apply editor --take-over"
Enter
Sleep 5s
Type@40ms "# the tracked copy preserves a real file for the application"
Enter
Sleep 2s
Type@40ms "cat ~/.config/helix/config.toml"
Enter
Sleep 4s
Type@40ms "grip generations"
Enter
Sleep 4s
Type@40ms "# rollback restores the retained previous state"
Enter
Sleep 2s
Type@40ms "grip rollback"
Enter
Sleep 4s
Type@40ms "cat ~/.config/helix/config.toml"
Enter
Sleep 4s
