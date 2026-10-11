//! Everything the pages reference, embedded so the container is one file.

pub struct Asset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub const ASSETS: &[Asset] = &[
    Asset {
        path: "/site.css",
        content_type: "text/css; charset=utf-8",
        bytes: include_bytes!("../assets/site.css"),
    },
    Asset {
        path: "/logo.png",
        content_type: "image/png",
        bytes: include_bytes!("../../../assets/brand/logo.png"),
    },
    Asset {
        path: "/mark.png",
        content_type: "image/png",
        bytes: include_bytes!("../../taskstaka/assets/mark.png"),
    },
    Asset {
        path: "/favicon.ico",
        content_type: "image/x-icon",
        bytes: include_bytes!("../../taskstaka/assets/favicon.ico"),
    },
    Asset {
        path: "/apple-touch-icon.png",
        content_type: "image/png",
        bytes: include_bytes!("../../taskstaka/assets/apple-touch-icon.png"),
    },
    Asset {
        path: "/shots/next.png",
        content_type: "image/png",
        bytes: include_bytes!("../assets/shots/next.png"),
    },
    Asset {
        path: "/shots/outline.png",
        content_type: "image/png",
        bytes: include_bytes!("../assets/shots/outline.png"),
    },
];
