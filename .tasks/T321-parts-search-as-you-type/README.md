# T321: The parts search filters as you type

The search box hands in what is typed only on Enter or a click elsewhere (`osp_ui::input`), so the
list does not change while typing. Players expect it to filter at each key.

## Done when

- The list filters at each key, Escape empties it, and a playtest finds an engine by typing part of
  its name without Enter.
