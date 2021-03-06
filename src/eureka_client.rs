use std::io;
use futures::{Future, Stream};
use serde_json;
use request::RegisterRequest;
use response::{ApplicationResponse, ApplicationsResponse};
use errors::EurekaClientError;
use hyper::{Client, Method, Request, Body, Uri, mime, Error as HyperError, StatusCode};
use hyper::header::{Accept, AcceptEncoding, Encoding, Headers, UserAgent, ContentType, ContentLength, AcceptCharset, Charset, qitem};
use tokio_core::reactor::Handle;

/// A client for accessing Eureka
pub struct EurekaClient<'a> {
    handle: &'a Handle,
    client_name: String,
    eureka_cluster_url: String,
}

//
// A simple port of the example found at: https://github.com/Netflix/eureka/wiki/Example-Custom-ReadOnly-client
// Eureka REST API: https://github.com/Netflix/eureka/wiki/Eureka-REST-operations
impl<'a> EurekaClient<'a> {

    /// Creates a new instance of EurekaClient
    ///
    /// # Arguments
    ///
    /// * `handle` - a Tokio Core handle
    /// * `client_name` - The name of this client
    /// * `eureka_cluster_url` - The base url to the eureka cluster
    pub fn new(handle: &'a Handle, client_name: &str, eureka_cluster_url: &str) -> EurekaClient<'a> {
        debug!("Creating new Eureka Client client_name:{:?}, eureka_client:{:?}", client_name, eureka_cluster_url);
        EurekaClient {
            handle: &handle,
            client_name: client_name.to_owned(),
            eureka_cluster_url: eureka_cluster_url.to_owned()
        }
    }

    pub fn register(&self, application_id: &str, register_request: &RegisterRequest) -> Box<Future<Item=(), Error=EurekaClientError>> {
        debug!("register: application_id={:?}, register_request:{:?}", application_id, register_request);
        let client = Client::new(self.handle);
        let path = "/v2/apps/".to_owned() + application_id;
        let mut req: Request<Body> = Request::new(Method::Post, self.build_uri(path.as_ref()));
        self.set_headers(req.headers_mut());

        let json = serde_json::to_string(register_request).unwrap();
        req.headers_mut().set(ContentLength(json.len() as u64));
        req.set_body(json);

        let result = client.request(req)
            .map_err(|e| {
                EurekaClientError::from(e)
            })
            .and_then(|res| {
                debug!("register: server response {:?}", res);

                let status = res.status();
                match status {
                    StatusCode::BadRequest => Err(EurekaClientError::BadRequest),
                    StatusCode::InternalServerError => Err(EurekaClientError::InternalServerError),
                    _ => Ok(())
                }
            });
        Box::new(result)
    }

    pub fn get_application<'b>(&self, application_id: &str) -> Box<Future<Item=ApplicationResponse, Error=EurekaClientError>> {
        // Since it was hard to coerce the errot type into a EurekaClientError
        // I set the result in a holder then map result into an error or ok
        // There has to be a better way.. but this works.
        enum IntermediateResult {
            Ok(ApplicationResponse),
            Err(EurekaClientError)
        }

        let client = Client::new(self.handle);
        let path = "/v2/apps/".to_owned() + application_id;
        let mut req: Request<Body> = Request::new(Method::Get, self.build_uri(path.as_ref()));
        self.set_headers(req.headers_mut());
        // for some reason gzip request works here but not when grabbing all applications
        // so we explicitly set it here instead of set_headers
        req.headers_mut().set(AcceptEncoding(vec![qitem(Encoding::Gzip)]));

        let result = client.request(req).and_then(|res| {
            let status = res.status();
            debug!("get_application_instances: server response {:?}", res);
            res.body().concat2().and_then(move |body| {
                match status {
                    StatusCode::NotFound => Ok(IntermediateResult::Err(EurekaClientError::NotFound)),
                    _ => {
                        serde_json::from_slice::<ApplicationResponse>(&body).map_err(|e| {
                            HyperError::Io(io::Error::new(io::ErrorKind::Other, e))
                        })
                            .map(|r| IntermediateResult::Ok(r))
                    }
                }
            })
        })
            .map_err(|e| {
                EurekaClientError::from(e)
            })
            .and_then(|ir| {
                // now that we have changed the error to EurekaClientError
                // we can map our err back in
                match ir {
                    IntermediateResult::Ok(app) => Ok(app),
                    IntermediateResult::Err(err) => Err(err)
                }
            });
        Box::new(result)
    }

    pub fn get_applications<'b>(&self) -> Box<Future<Item=ApplicationsResponse, Error=EurekaClientError>> {
        // Since it was hard to coerce the errot type into a EurekaClientError
        // I set the result in a holder then map result into an error or ok
        // There has to be a better way.. but this works.

        #[derive(Debug)]
        enum IntermediateResult {
            Ok(ApplicationsResponse),
            Err(EurekaClientError)
        }

        let client = Client::new(self.handle);
        let path = "/v2/apps";
        let uri = self.build_uri(path.as_ref());
        debug!("get_applications uri:{}", uri);
        let mut req: Request<Body> = Request::new(Method::Get, uri);
        self.set_headers(req.headers_mut());

        let result = client.request(req).and_then(|res| {
            let status = res.status();
            debug!("get_applications_instances: server response {:?}", res);
            res.body().concat2().and_then(move |body| {
                match status {
                    StatusCode::NotFound => {
                        debug!("received NotFound (404) from server");
                        Ok(IntermediateResult::Err(EurekaClientError::NotFound))
                    }
                    _ => {
                        serde_json::from_slice::<ApplicationsResponse>(&body).map_err(|e| {
                            warn!("serde error: {:?}", e);
                            HyperError::Io(io::Error::new(io::ErrorKind::Other, e))
                        })
                            .map(|r| IntermediateResult::Ok(r))
                    }
                }
            })
        })
            .map_err(|e| {
                EurekaClientError::from(e)
            })
            .and_then(|ir| {
                // now that we have changed the error to EurekaClientError
                // we can map our err back in
                match ir {
                    IntermediateResult::Ok(app) => {
                        debug!("returning: {:?}", app);
                        Ok(app)
                    }
                    IntermediateResult::Err(err) => {
                        debug!("returning err: {}", err);
                        Err(err)
                    }
                }
            });
        Box::new(result)
    }

    fn build_uri(&self, path: &str) -> Uri {
        (self.eureka_cluster_url.to_owned() + path).parse().unwrap()
    }

    fn set_headers(&self, headers: &mut Headers) {
        headers.set(Accept(vec![qitem(mime::APPLICATION_JSON)]));
        headers.set(ContentType(mime::APPLICATION_JSON));
        headers.set(AcceptCharset(vec![qitem(Charset::Ext("utf-8".to_owned()))]));
        let user_agent = "Rust Hyper/".to_string() + self.client_name.as_ref();
        headers.set(UserAgent::new(user_agent));
    }
}