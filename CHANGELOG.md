# Samply.Beam 0.3.0

We are happy to announce the new Samply.Beam major release 0.3.0. Like many early-stage software projects, the Samply.Beam developments moves fast and we are exited to see new features, improved performance and stability, and an overall improvement of code quality and error handling. Furthermore, the first productive use of Samply.Beam shoed some shortcomings in the API, so we strived to streamline the developer experience for costumer applications.

Unfortunately, this high pace of development and the significant improvements come with some breaking changes in the API. If you have not already, please adapt your applications accordingly.

There are still some interesting features planned and scheduled, some of which will introduce another breaking API change. However, we expect to reach a more stable state in a few months.

Thank you for using Samply.Beam and all your feedback, helping us improve Samply.Beam further.

## Breaking changes

### Expiry of messages

Many tasks are only relevant for a specific duration. This is now reflexted in the mandatory `ttl` field of the task JSON, stating the ttime-to-live in seconds. After this time, the task and all its attached results expire and are cleaned up.

### Removed `status` body for `Result` objects

### Cleaned up public API
 * Removed `id` field from result
 * Changed result endpoint
 
## Major changes

### The Beam.Proxies now address the central CA via Beam.Broker

By addessing the central CA via central Broker the configuration of the local Beam.Proxies is greatly reduced and the central CA does not need to be publicly reachable.

### Support for TLS terminating proxy servers

As many institutions operate a Man-in-the-middle TLS-terminating proxy, you can now provide the appropriate certificate to sucessfully establish connections through theses systems. The optional commandline parameter `--tls_ca_certificates_dir` (or the respective environment variable) takes the folder containing all relevant certificates.

### Respect `no_proxy` environment variable

Due to the proxy traversal system overhaul, Beam.Broker and Beam.Proxy now correctly uses the `no_proxy` environment variable. As before, `http_proxy` and `https_proxy`, or as a new option `all_proxy`, are used for proxy usage.

### Cross compilation CI/CD pipeline for arm64 images

A new, higher performance CI/CD pipeline automatically builds Beam docker images not only for amd64, but for arm64 architectures as well.

## Minor improvements

 * Show an informational banner at startup
 * Respect the `RUST_LOG` variable for log level
 * Improved testing
 * Internal improvements
