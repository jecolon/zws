# zws
An multithreaded HTTP2 / TLS web server written in Rust

## Usage
```sh
Usage: zws [-h] [-c CERT] [-k KEY] [-s SOCKET] [-t THREADS] [-w DIR]

Options:
    -h, --help
        Show this usage screen.

    -c CERT, --cert CERT
        Path to PEM certificate file. [default: tls/dev/cert.pem]

    -k KEY, --key KEY
        Path to PEM key file. [default: tls/dev/key.pem]

    -s SOCKET, --socket SOCKET
        TCP socket to listen on. [default: 127.0.0.1:8443]

    -t THREADS, --threads THREADS
        Number of threads for worker pool request handling.
        0 = Total logical CPUs. [default: 0]

    -w DIR, --webroot DIR
        Path to root of file serving area. [default: webroot]
```
