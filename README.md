# zws
An multithreaded HTTP2 / TLS web server written in Rust

## Setup
By default, the server looks for certificate and key files in PEM format in a
directory named `tls` from where you run the executable. Within that directory,
sub-directories named `dev` and `prod` can separate local testing cert and key
files from production ones. You also need a webroot to serve static files from.
By default, this is a directory neamed `webroot` in the same directory from 
which you run the executable.

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
