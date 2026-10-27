package tunnel

import (
	"errors"
	"io"
	"net"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

type streamConn struct {
	ws     *websocket.Conn
	reader io.Reader
	mu     sync.Mutex
}

func (c *streamConn) Read(data []byte) (int, error) {
	if len(data) == 0 {
		return 0, nil
	}
	for {
		if c.reader != nil {
			n, err := c.reader.Read(data)
			if n > 0 {
				return n, nil
			}
			if err == nil {
				continue
			}
			if err != io.EOF {
				return 0, err
			}
			c.reader = nil
		}
		kind, reader, err := c.ws.NextReader()
		if err != nil {
			if websocket.IsCloseError(err, websocket.CloseNormalClosure, websocket.CloseGoingAway) {
				return 0, io.EOF
			}
			return 0, err
		}
		if kind != websocket.BinaryMessage {
			return 0, errors.New("binary publish frames required")
		}
		c.reader = reader
	}
}

func (c *streamConn) Write(data []byte) (int, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	written := 0
	for len(data) > 0 {
		n := min(len(data), 16<<10)
		if err := c.ws.WriteMessage(websocket.BinaryMessage, data[:n]); err != nil {
			return written, err
		}
		written += n
		data = data[n:]
	}
	return written, nil
}

func (c *streamConn) Close() error         { return c.ws.Close() }
func (c *streamConn) LocalAddr() net.Addr  { return c.ws.UnderlyingConn().LocalAddr() }
func (c *streamConn) RemoteAddr() net.Addr { return c.ws.UnderlyingConn().RemoteAddr() }
func (c *streamConn) SetDeadline(t time.Time) error {
	if err := c.ws.SetReadDeadline(t); err != nil {
		return err
	}
	return c.ws.SetWriteDeadline(t)
}
func (c *streamConn) SetReadDeadline(t time.Time) error  { return c.ws.SetReadDeadline(t) }
func (c *streamConn) SetWriteDeadline(t time.Time) error { return c.ws.SetWriteDeadline(t) }
