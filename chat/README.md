# Simple Chat

This is a simple, one-room chat application that can handle multiple clients.
Once the server is running and a client connects, the user will be prompted for
a unique username (if one was not provided as a startup argument).  Then, every
message the user types must be of the form 'leave' or 'send <msg>'.  Every time
the user types 'send <msg>' the message will be sent to all other connected
clients.  Any messages from other clients will show their username as well as
their message to the screen.

![Demo video](static/ddn-simple-chat-demo.mkv)
