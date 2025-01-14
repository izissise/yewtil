//! Feature to enable fetching using web_sys-based fetch requests.
//!
//! This feature makes use of JSON for the request and response bodies.
//!
//! # Note
//! Because this makes use of futures, enabling this feature will require the use of a
//! web_sys compatible build environment and will prevent you from using `cargo web`.


use crate::NeqAssign; // requires "neq" feature.

mod action;
mod error;
mod state;
mod request;

pub use self::action::*;
pub use self::error::*;
pub use self::state::*;
pub use self::request::*;

/// Indicates that a change was caused by a set function.
pub type DidChange = bool;


/// A fetch type that is useful for when you don't hold any request directly.
///
/// This is useful for GET and DELETE requests where additional information needed to create the request object
/// can be provided by a closure.
pub type AcquireFetch<T> = Fetch<(), T>;

/// A fetch type that is useful for when the request type is the same as the response type.
///
/// This makes sense to use when the request and response bodies are exactly the same.
/// Some PUT requests are amenable to this arrangement.
pub type ModifyFetch<T> = Fetch<T, T>;


#[derive(Clone, Debug, PartialEq, Default)]
pub struct Fetch<REQ, RES> {
    request: REQ,
    response: FetchState<RES>
}


impl <REQ: PartialEq, RES> Fetch<REQ, RES> {

    /// Sets the request without changing the variant.
    pub fn set_req(&mut self, request: REQ) -> DidChange {
        self.request.neq_assign(request)
    }
}

impl <REQ: Default, RES: PartialEq> Fetch<REQ, RES> {

    /// Sets the Fetch wrapper to indicate that a request was successfully fetched.
    pub fn set_fetched(&mut self, res: RES) -> DidChange {
        let will_change = match &self.response {
            FetchState::Fetched(old_res) => {
                &res == old_res
            },
            _ => true
        };

        // TODO replace this with std::mem::take when it stabilizes.
        let old = std::mem::replace(&mut self.response, FetchState::default());
        let new = old.fetched(res);
        std::mem::replace(&mut self.response, new);

        will_change
    }

    /// Apply a FetchAction to alter the Fetch wrapper to perform a state change.
    pub fn apply(&mut self, action: FetchAction<RES>) -> DidChange {
        match action {
            FetchAction::NotFetching => self.set_not_fetching(),
            FetchAction::Fetching => self.set_fetching(),
            FetchAction::Success(res) => self.set_fetched(res),
            FetchAction::Failed(err) => self.set_failed(err),
        }
    }
}

impl <REQ, RES> Fetch<REQ, RES> {
    /// Creates a new Fetch wrapper around the request.
    ///
    /// It will default the response field to be put in a NotFetching state.
    pub fn new(request: REQ) -> Self {
        Self {
            request,
            response: Default::default()
        }
    }

    /// Sets the response field to indicate that no fetch request is in flight.
    pub fn set_not_fetching(&mut self) -> DidChange {
        let will_change = self.response.discriminant_differs(&FetchState::NotFetching(None));

        let old = std::mem::replace(&mut self.response, FetchState::default());
        let new = old.not_fetching();
        std::mem::replace(&mut self.response, new);

        will_change
    }

    /// Sets the response field to indicate that a fetch request is currently being made.
    pub fn set_fetching(&mut self) -> DidChange {
        let will_change = self.response.discriminant_differs(&FetchState::Fetching(None));

        let old = std::mem::replace(&mut self.response, FetchState::default());
        let new = old.fetching();
        std::mem::replace(&mut self.response, new);

        will_change
    }

    /// Sets the response field to indicate that a fetch request failed to complete.
    pub fn set_failed(&mut self, err: FetchError) -> DidChange {
        let will_change = match &self.response {
            FetchState::Failed(_, old_err) => {
                &err == old_err
            }
            _ => true
        };

        let old = std::mem::replace(&mut self.response, FetchState::default());
        let new = old.failed(err);
        std::mem::replace(&mut self.response, new);

        will_change
    }



    // TODO need tests to make sure that this is ergonomic.
    /// Makes an asynchronous fetch request, which will produce a message that makes use of a
    /// `FetchAction` when it completes.
    pub async fn fetch_convert<T: FetchRequest, Msg>(
        &self,
        to_request: impl Fn(&Self) -> &T,
        to_msg: impl Fn(FetchAction<T::ResponseBody>) -> Msg
    ) -> Msg {
        let request = to_request(self);
        let fetch_state = match fetch_resource(request).await {
            Ok(response) => FetchAction::Success(response),
            Err(err) => FetchAction::Failed(err)
        };

        to_msg(fetch_state)
    }

    /// Transforms the type of the response held by the fetch state (if any exists).
    pub fn map<NewRes, F: Fn(Fetch<REQ, RES>) -> Fetch<REQ, NewRes> >(self, f:F) -> Fetch<REQ, NewRes> {
        f(self)
    }

    /// Unwraps the Fetch wrapper to produce the response it may contain.
    ///
    /// # Panics
    /// If the Fetch wrapper doesn't contain an instance of a response, this function will panic.
    pub fn unwrap(self) -> RES {
        // TODO, actually provide some diagnostic here.
        self.res().unwrap()
    }

    /// Gets the response body (if present).
    pub fn res(self) -> Option<RES> {
        match self.response {
            FetchState::NotFetching(res) => res,
            FetchState::Fetching(res) => res,
            FetchState::Fetched(res) => Some(res),
            FetchState::Failed(res, _) => res,
        }
    }

    /// Gets the request body.
    pub fn req(self) -> REQ {
        self.request
    }

    /// Converts the wrapped values to references.
    ///
    /// # Note
    /// This may be expensive if a Failed variant made into a reference, as the FetchError is cloned.
    pub fn as_ref(&self) -> Fetch<&REQ, &RES> {
        let response = match &self.response {
            FetchState::NotFetching(res) => FetchState::NotFetching(res.as_ref()),
            FetchState::Fetching(res) => FetchState::Fetching(res.as_ref()),
            FetchState::Fetched(res) => FetchState::Fetched(res),
            FetchState::Failed(res, err) => FetchState::Failed(res.as_ref(), err.clone()),
        };

        Fetch {
            request: &self.request,
            response
        }
    }

    /// Converts the wrapped values to mutable references.
    ///
    /// # Note
    /// This may be expensive if a Failed variant made into a reference, as the FetchError is cloned.
    pub fn as_mut(&mut self) -> Fetch<&mut REQ, &mut RES> {
        let response = match &mut self.response {
            FetchState::NotFetching(res) => FetchState::NotFetching(res.as_mut()),
            FetchState::Fetching(res) => FetchState::Fetching(res.as_mut()),
            FetchState::Fetched(res) => FetchState::Fetched(res),
            FetchState::Failed(res, err) => FetchState::Failed(res.as_mut(), err.clone()),
        };
        Fetch {
            request: &mut self.request,
            response
        }
    }
}

impl <REQ: FetchRequest> Fetch<REQ, REQ::ResponseBody>{

    /// Makes an asynchronous fetch request, which will produce a message that makes use of a
    /// `FetchAction` when it completes.
    pub async fn fetch<Msg>(
        &self,
        to_msg: impl Fn(FetchAction<REQ::ResponseBody>) -> Msg
    )-> Msg {
        let request = self.as_ref().req();
        let fetch_state = match fetch_resource(request).await {
            Ok(response) => FetchAction::Success(response),
            Err(err) => FetchAction::Failed(err)
        };

        to_msg(fetch_state)
    }
}


#[cfg(test)]
mod test {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn setting_fetching_state_doesnt_change_strong_count() {
        // This is done to detect if a leak occurred.
        let data: Arc<i32> = Arc::new(22);
        let cloned_data: Arc<i32> = data.clone();
        assert_eq!(Arc::strong_count(&data), 2);
        let mut fs: Fetch<Arc<i32>, ()> = Fetch::new(cloned_data);
        fs.set_fetching();

        assert_eq!(Arc::strong_count(&data), 2);
        assert_eq!(FetchState::Fetching(None), fs.response)
    }

    #[test]
    fn setting_fetched_state() {
        let mut fs = Fetch {
            request: (),
            response: FetchState::Fetching(None)
        };
        assert!(fs.set_fetched("SomeValue".to_string()));
        assert_eq!(fs.response, FetchState::Fetched("SomeValue".to_string()));
    }

    #[test]
    fn setting_fetching_from_fetched() {
        let mut fs = Fetch {
            request: (),
            response: FetchState::Fetched("Lorem".to_string())
        };
        assert!(fs.set_fetching());
        assert_eq!(fs.response, FetchState::Fetching(Some("Lorem".to_string())));
    }
}
