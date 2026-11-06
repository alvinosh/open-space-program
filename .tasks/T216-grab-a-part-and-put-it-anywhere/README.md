# T216: Grab a placed part and put it on anywhere

Asked for on 2026-11-06, in one list of what is wrong with the editor: "the left hand side parts
menu is just full of text and not intuitive at all, there is overlap issues, also 5 buttons at the
top, all with text, not intuitive at all, also when you click on a part a menu comes up, we should
use the left hand side parts menu and display the information there [...] i want to be able to grab
a part and move it [...] we also need to add symetry modes, mirror modes, part clipping by
different offets, like 1degree 5degreee 20 degree 45 ect. And we should use icons where we can."
That list is T207 and T212 to T218.

The example given: "I should be able to move a tank from the bottom of the pod to the top."

A placed part can already be dragged off with what it holds (`building`), but not the root, which
turns the view instead. Moving a part that holds others to a node on the other side also means
joining it by another of its nodes. Find out first what goes wrong in the given case.

## Scope

- Reproduce the case in a capture: pod, tank under it, engine under that; drag the tank (with or
  without the engine) to the pod's top.
- A grabbed part goes on by whichever of its free nodes faces the node it is brought to; the parts it
  holds come along.
- The root can be grabbed too: the craft is rerooted to what stays (`Builder::reroot`), as KSP's
  re-root tool does, or the root is made the grabbed part's holder. Pick one and write it here.
- Symmetric copies go along (`builder::alike`). Undo, redo, and the rules checked with the reason
  shown when refused.

## Done when

- In a capture, a tank under a pod is dragged to the top of the pod and stays there, then the pod
  (the root) is dragged under the tank, and both moves can be undone.
