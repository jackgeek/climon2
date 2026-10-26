# Detached session daemon from the first feature

Feature 1, `exec` plus discovery, already runs each session in a detached session daemon. `exec` then attaches to it as the first client, even though feature 1 still ends the session when that client disconnects. Starting with the final process shape means that detach, attach, reclaim and screen restore are added by amending behaviour, not by rearchitecting `exec`. It also means discovery tests validate the real daemon and handshake from the start.
