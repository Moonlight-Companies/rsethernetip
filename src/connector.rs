//! Creates and tears down EtherNet/IP sessions for the transport layer.
//!
//! Port of Go `connector.go`.

use std::sync::Arc;

use crate::conn::{Result, TcpConn};
use crate::logging::{Logger, NopLogger};
use crate::options::ConnOption;
use crate::plc::Context;
use crate::session::Session;
use crate::transport::{Closer, Connector};

// Ref: CIP Vol. 2, Section 2-4 RegisterSession (section number unverified): a session is
// established by dialing TCP and registering.
// Implementation detail: implements `transport::Connector<Arc<Session>>`; used internally by
// `connect` and `new_reconnecting_client`.
/// Creates new EtherNet/IP sessions by dialing TCP and sending RegisterSession.
pub struct SessionConnector {
    address: String,
    logger: Arc<dyn Logger>,
    conn_opts: Vec<ConnOption>,
}

impl SessionConnector {
    /// Creates a connector that will dial `address`. If `logger` is `None` a no-op
    /// logger is used.
    pub fn new(address: impl Into<String>, logger: Option<Arc<dyn Logger>>, conn_opts: Vec<ConnOption>) -> SessionConnector {
        SessionConnector {
            address: address.into(),
            logger: logger.unwrap_or_else(|| Arc::new(NopLogger::new())),
            conn_opts,
        }
    }

    /// Dials TCP, creates a [`Session`], and sends RegisterSession to obtain a session
    /// handle from the target.
    pub fn connect(&self, ctx: &Context) -> Result<Arc<Session>> {
        let tc = TcpConn::new(&self.address, &self.conn_opts)?;

        let s = Session::new(tc, Some(self.logger.clone()));
        if let Err(e) = s.register(ctx) {
            let _ = s.close();
            return Err(e);
        }

        Ok(Arc::new(s))
    }
}

impl Connector<Arc<Session>> for SessionConnector {
    fn connect(&self, ctx: &Context) -> Result<Arc<Session>> {
        SessionConnector::connect(self, ctx)
    }
}

// Ref: CIP Vol. 2, Section 2-4 UnregisterSession (section number unverified).
/// Tears down a session by sending UnregisterSession and closing the TCP connection.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionCloser;

impl SessionCloser {
    /// Sends UnregisterSession (errors ignored, as in Go) and closes the TCP connection.
    pub fn close(&self, sess: &Session) -> Result<()> {
        // Use a background context since close has no ctx parameter.
        let _ = sess.unregister(&Context::background());
        sess.close()
    }
}

impl Closer<Arc<Session>> for SessionCloser {
    fn close(&self, conn: &Arc<Session>) -> Result<()> {
        SessionCloser::close(self, conn)
    }
}
