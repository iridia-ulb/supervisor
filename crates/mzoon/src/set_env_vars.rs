use crate::config::Config;
use std::env;

pub fn set_env_vars(config: &Config, release: bool) {
    env::set_var("ADDRESS", config.address.to_string());
    // port = 8443
    env::set_var("PORT", config.port.to_string());
    // https = true
    env::set_var("HTTPS", config.https.to_string());
    // cache_busting = true
    env::set_var("CACHE_BUSTING", config.cache_busting.to_string());

    // [redirect]
    // port = 8080
    env::set_var("REDIRECT_PORT", config.redirect.port.to_string());
    // enabled = true
    env::set_var("REDIRECT_ENABLED", config.redirect.enabled.to_string());

    env::set_var("COMPRESSED_PKG", release.to_string());
}
