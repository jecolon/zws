pub mod server;

#[cfg(test)]
mod tests {
    use crate::server;

    struct T<'a> {
        filename: &'a str,
        want: &'a str,
    }

    #[test]
    fn ctype_workds() {
        let tests = vec![
            T {
                filename: "a.html",
                want: "text/html; charset=utf-8",
            },
            T {
                filename: "a.css",
                want: "text/css",
            },
            T {
                filename: "a.js",
                want: "text/javascript",
            },
            T {
                filename: "a.png",
                want: "image/png",
            },
            T {
                filename: "a.jpg",
                want: "image/jpeg",
            },
            T {
                filename: "a.gif",
                want: "image/gif",
            },
            T {
                filename: "a.svg",
                want: "image/svg+xml",
            },
            T {
                filename: "a.webp",
                want: "image/webp",
            },
            T {
                filename: "a.txt",
                want: "text/plain; charset=utf-8",
            },
            T {
                filename: "a.json",
                want: "application/json",
            },
            T {
                filename: "a.exe",
                want: "binary/octet-stream",
            },
        ];

        for t in tests {
            assert_eq!(server::get_ctype(t.filename), t.want);
        }
    }
}
