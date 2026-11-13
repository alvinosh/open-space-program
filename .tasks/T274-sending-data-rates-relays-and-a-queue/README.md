# T274: Sending data: rates, relays and a queue

A link's data rate follows its margin, so it falls with distance; through relays the slowest hop sets
it, and vessels on one relay share it. Each vessel sends its drives in order, its own state first, in
the background, pausing when the link goes and going on when it comes back.

Casual: data arrives as soon as there is a link. Realistic: the rates.

## Done when

- A file takes its size over the rate, and the rate falls with distance (test).
- Losing the link pauses a file and nothing is lost when it resumes (test).
- Two probes through one relay share its rate (test).

## Done (2026-11-13)

- A link's rate (`comms::rate`): its radio's `data_rate` with 10 dB of margin to spare
  (`FULL_RATE_MARGIN`), slower below that as the square of the distance, a tenth at the link's edge.
  Where a link reaches is unchanged. Through relays the slowest hop sets it (`Link::rate`, and
  `Link::relays`, the path), and everyone sending through a relay shares what it passes on, its own
  data included (`comms::shared_rates`).
- Sending (`Universe::send_data`, `send_over`): every vessel with data on its drives and a radio on
  sends, in the background, flown or not. Each physics tick sends at the link as it last was (looked
  at once a second). A jump on rails works the links out again at the middle of spans of at most
  600 s or 1/360 of an orbit, so a pass over a station sends what it does at any warp (one long jump
  and jumps of ten seconds over two low orbits differ by 0.2%). The queue: drive by drive, subject by
  subject. What goes leaves the drive, which frees room for experiments, and counts in a career's
  `returned` as recovery does.
- Casual and Realistic: rules `data_rates`. Off (the base game), all of a vessel's data goes at once
  whenever it has a link. On, the rates; a new mod, Realistic Communications (`realistic_comms`), sets
  it and is in the Realistic game mode.
- The Communications window shows the link's data rate (with rates), the data waiting on the drives
  (also with no radio aboard) and how long it takes to send.
- Tests (`comms::tests::sending`): the rate falls with distance past the margin to spare; 50 kB takes
  50 kB over the rate and comes home a whole set; without rates it all goes with the link and nothing
  without one; losing the link pauses sending and nothing is lost, the same at any warp; two probes
  through one relay share its rate, a third each with the relay sending too. Capture:
  `OSP_CAPTURE_SCIENCE` ends on the Communications window of the capsule, with no radio and 0.15 kB
  waiting.

## Left out

- Telemetry ahead of science (spec 7.9): the radio's `data_rate` is all for data.
- Solar storms (T269) and antenna failures (T050).
- Time warp does not stop when a vessel's data has all gone, or where its link changes (unless the
  player asked for the flown one's).
- The rate rows of the Communications window were not seen in the game: the capture's capsule has no
  antenna.
