Playing with FIFO named pipes.

### Daemon bin to:
* make & blocking read at /tmp/pw_bridge_requests

### CLI bin to:
* create its own dedicated FIFO in /tmp/
* write Request to the daemon, passing its FIFO
* Read Response from daemon, on its FIFO
