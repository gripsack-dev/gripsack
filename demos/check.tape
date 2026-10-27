# gripsack CLI demo — a workspace reference error with a source location,
# fixed, then accepted. Neither check activates files.
# Local render: see .agents/skills/gripsack-demo-capture.
Output demos/demo-check.gif
Set FontSize 28
Set Width 1800
Set Height 1120
Set Theme "Catppuccin Mocha"

Type@40ms "# a workspace with a misspelled environment reference"
Enter
Sleep 3s
Type@40ms "sh /vhs/demos/check_setup.sh && cd /tmp/checkenv"
Enter
Sleep 4s
Type@40ms "# check catches it before anything stages"
Enter
Sleep 2s
Type@40ms "grip check"
Enter
Sleep 4s
Type@40ms "y"
Enter
Sleep 4s
Type@40ms "# fix the typo"
Enter
Sleep 2s
Type@40ms "sed -i 's/TYPO/dev/' gripsack.ts"
Enter
Sleep 4s
Type@40ms "# green - the CI gate for your dotfiles repo"
Enter
Sleep 2s
Type@40ms "grip check"
Enter
Sleep 4s
Type@40ms "y"
Enter
Sleep 4s
