# Samply.Beam 0.4.0 --

This newest release of Samply.Beam paves the road to more production safety. We revised the CA deployment and usage, established a clearly defined (and tool supported) certificate enrollment process, and improved the overall stability of the applications. Additionally, the end-to-end encryption is fully operational in this version.

The new certificate and private key handling process requires manual intervention in the update process, so please carefully read the following change notice.

We are exited to release this new version and help with shaping the infrastructure landscape of the European medical informatics.

## Breaking changes

### Certificate enrollment

In previous releases of Beam, certificate and private key generation was entirely left to the system administrator. Furthermore, only the certificate expiry but not the certificate chain validated. Now, both the Beam.Proxy and the Beam.Broker require the CA's root certificate to be present locally and validates the Root CA --> Intermediate CA --> Proxy Certificate chain. For that, the command line parameter `--rootcert-file`, and the environment variable `ROOTCERT_FILE` have been introduced. The [Beam deployment packages]() reflect this change by loading the root certificate via the docker-compose secret mechanism.

Furthermore, the process of creating a private key and enrolling a matching certificate into the CA has been described in the documentation and a companion tool to assist system administrators [has been published](). The companion tool takes the proxy id and broker id and a) generates a private key in a location of choice and b) prints out a Certificate Sign Request (CSR), that can be signed at the central CA. This way, we hope to ensure, that the private key never leaves the local sites.

You can find more information in the [documentation](README.MD#beam-enrollment-companion-tool-assisted-method) and in the repository for [Beam's central docker-compose deployment]().

### Certificate validation and selection

Previously, if multiple certificates for a single common name were present in the CA the first one retrieved was chosen by the Beam.Proxy to use. In this version, the logic of choosing a certificate has changed: First, expired certificates are discarded. Second, certificates using a different modulus than the private key are discarded, and lastly, the newest of the remaining certificates are chosen.

Future versions of Beam will include an additional Certificate Revocation List check, however, this is not implemented, yet.

If your current setup relies on the previous behaviour, please check your CA and setup the certificates to follow the previously outlined selection rules.

### End-to-end encryption

This version fullz implements end/to/end/encrzption between the Beam.Proxies. This breaks downwards compatibility and requires all Beam.Proxies, as well as the central Beam.Broker, to run at least this version (v0.4.0).

## Major changes

* Bugfix: `Wait_count` malfunction. There was an issue, where some results were not returned by the long polling mechanism. The issue is fixed in this version.

## Minor improvement

* Improved debug output and logging
* Updated dependencies
* Removed `vault-rs` dependency

# Samply.Beam 0.3.0 -- 2022-09-26

We are happy to announce the new Samply.Beam major release 0.3.0. Like many early-stage software projects, the Samply.Beam developments moves fast and we are exited to see new features, improved performance and stability, and an overall improvement of code quality and error handling. Furthermore, the first productive use of Samply.Beam brought some shortcomings in the API to our attention, so we strived to streamline the developer experience for costumer applications.

Unfortunately, this high pace of development and the significant improvements come with some breaking changes in the API. If you have not already, please adapt your applications accordingly.

There are still some interesting features planned and scheduled, some of which will introduce another breaking API change. However, we expect to reach a more stable state in a few months.

Thank you for using Samply.Beam and all your feedback, helping us improve Samply.Beam further.

## Breaking changes

### Expiry of messages

Many tasks are only relevant for a specific duration. This is now reflected in the mandatory `ttl` field of the task JSON, stating the time-to-live in seconds. After this time, the task and all its attached results expire and are cleaned up.

### Removed `status` body for `Result` objects

To be consistent in the usage of the `Task` and `Result` JSONs, we changed the
`Result` interface. The `status` now only provides the status as a string (i.e.,
`claimed`, `succeeded`, `tempfailed`, or `permfailed`) and does not take the
return body itself. Instead, the `body` of a successful result is attached to
the root of the `Result` object.

### Cleaned up public API

* Removed `id` field from result
* Changed result endpoint

## Major changes

### The Beam.Proxies now address the central CA via Beam.Broker

By addressing the central CA via central Broker the configuration of the local Beam.Proxies is greatly reduced and the central CA does not need to be publicly reachable.

### Support for TLS terminating proxy servers

As many institutions operate a Man-in-the-middle TLS-terminating proxy, you can now provide the appropriate certificate to successfully establish connections through theses systems. The optional command line parameter `--tls_ca_certificates_dir` (or the respective environment variable) takes the folder containing all relevant certificates.

### Respect `no_proxy` environment variable

Due to the proxy traversal system overhaul, Beam.Broker and Beam.Proxy now correctly uses the `no_proxy` environment variable. As before, `http_proxy` and `https_proxy`, or as a new option `all_proxy`, are used for proxy usage.

### Cross compilation CI/CD pipeline for arm64 images

A new, higher performance CI/CD pipeline automatically builds Beam docker images not only for amd64, but for arm64 architectures as well.

## Minor improvements

* Show an informational banner at startup
* Respect the `RUST_LOG` variable for log level
* Improved testing
* Internal improvements
