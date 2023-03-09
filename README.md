# Adobe Connect Room Status Monitor Backend

> This is not widely applicable software. It's just open source for education
> purposes; i.e., in case you want to try something similar for your
> institution.

## Overview/Context

Adobe Connect (AC) is a platform for online meetings, and it's what my school
uses for just about everything. Despite that, there is no single way to check
the status of a room in the event of a technical difficulty. This repository
contains (part of) some software to connect to the server running Adobe Connect
and monitor the status of a whole bunch of rooms. It publishes this data over a
REST API for a frontend to consume, aggregate, and display.

## Some Implementation Details for Future Reference

Joining an Adobe Connect room is actually quite complex! Each course Canvas
page contains a unique link to the class section's meeting room, which encodes
all sorts of data; primarily who you are and what class you're trying to join.
It redirects you around a few times before landing on a page that, in a
browser, decides how to launch AC: It uses some javascript to determine exactly
what version (flash, WebRTC, &c) of AC to open. However, this code isn't a
browser emulator - we don't run the JS. Instead, we extract out a special
ticket value (among a few other magic strings) and connect to the central AWS
AC server which provides, among other things, a websocket. Using the ticket
from before, we can pretend to join the room by sending over an RPC call that
subscribes us to updates. Then, we read the RPC calls it sends back and (ab)use
them to update ourselves on the status of the room. While all this is happening
for, well, every room in the school, we run an HTTP server and publish all we
know to `/api/v1/all`, which anyone can GET for a convenient JSON summary.

## Copying

You're welcome to steal code from this to monitor your own AC rooms - just make
sure to comply with the Mozilla Public License copyleft.

(C) Tali Auster, 2023.
