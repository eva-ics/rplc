use opcua::client::prelude::*;
use parking_lot::{Mutex, MutexGuard, RwLock};
use std::error::Error;
use std::sync::Arc;

#[allow(clippy::module_name_repetitions)]
pub type OpcSafeSession = Arc<OpcSafeSess>;

type OpcSession = Arc<RwLock<Session>>;

pub struct OpcSafeSess {
    client: Mutex<Client>,
    endpoint_description: EndpointDescription,
    user_identity_token: IdentityToken,
    session: Mutex<Option<OpcSession>>,
}

impl OpcSafeSess {
    pub fn new<T>(client: Client, endpoint: T, user_identity_token: IdentityToken) -> Self
    where
        T: Into<EndpointDescription>,
    {
        Self {
            client: Mutex::new(client),
            endpoint_description: endpoint.into(),
            user_identity_token,
            session: <_>::default(),
        }
    }
    pub fn reconnect(&self) {
        self.session.lock().take();
    }
    fn get_session(&self) -> Result<MutexGuard<Option<OpcSession>>, std::io::Error> {
        let mut lock = self.session.lock();
        if lock.as_mut().is_none() {
            let session = self.client.lock().connect_to_endpoint(
                self.endpoint_description.clone(),
                self.user_identity_token.clone(),
            )?;
            lock.replace(session);
        }
        Ok(lock)
    }
    /// # Panics
    ///
    /// Should not panic
    pub fn read(
        &self,
        nodes_to_read: &[ReadValueId],
        timestamps_to_return: TimestampsToReturn,
        max_age: f64,
    ) -> Result<Result<Vec<DataValue>, StatusCode>, Box<dyn Error>> {
        let mut session = self.get_session()?;
        let result =
            session
                .as_mut()
                .unwrap()
                .read()
                .read(nodes_to_read, timestamps_to_return, max_age);
        Ok(result)
    }
    /// # Panics
    ///
    /// Should not panic
    pub fn write(
        &self,
        nodes_to_write: &[WriteValue],
    ) -> Result<Result<Vec<StatusCode>, StatusCode>, Box<dyn Error>> {
        let mut session = self.get_session()?;
        let result = session.as_mut().unwrap().read().write(nodes_to_write);
        Ok(result)
    }
}
