# T213: Editor panels overlap each other

Asked for on 2026-11-06, in one list of what is wrong with the editor: "the left hand side parts
menu is just full of text and not intuitive at all, there is overlap issues, also 5 buttons at the
top, all with text, not intuitive at all, also when you click on a part a menu comes up, we should
use the left hand side parts menu and display the information there [...] i want to be able to grab
a part and move it [...] we also need to add symetry modes, mirror modes, part clipping by
different offets, like 1degree 5degreee 20 degree 45 ect. And we should use icons where we can."
That list is T207 and T212 to T218.

Seen in `OSP_CAPTURE_EDITOR_BUILD` (`build_part_window.png`): the part window opens over the left
panel and hides its top row, so the Shape button is cut off behind it. Most of it goes with T207,
which takes the popup away; this task is for what is left.

## Scope

- Check every editor state (empty hangar, list of vessels, parts list, a part picked, a part in
  hand, the staging window docked) at 1600x900, 1280x720 and a tall narrow window
  (`OSP_CAPTURE_SIZE`), and fix what overlaps or is cut off.

## Done when

- No two editor panels overlap, and no text is cut off, in captures at those three sizes.
