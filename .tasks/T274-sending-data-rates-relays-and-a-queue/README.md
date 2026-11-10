# T274: Sending data: rates, relays and a queue

A link's data rate follows its margin, so it falls with distance; through relays the slowest hop sets
it, and vessels on one relay share it. Each vessel sends its drives in order, its own state first, in
the background, pausing when the link goes and going on when it comes back.

Casual: data arrives as soon as there is a link. Realistic: the rates.

## Done when

- A file takes its size over the rate, and the rate falls with distance (test).
- Losing the link pauses a file and nothing is lost when it resumes (test).
- Two probes through one relay share its rate (test).
