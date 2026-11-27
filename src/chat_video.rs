use anyhow::{Context, Result};
use std::io::Write;
use url::Url;
use wry::{
    WebContext, WebView, WebViewBuilder,
    dpi::{LogicalPosition, LogicalSize},
};

pub struct Player {
    web: WebView,
    _page: tempfile::NamedTempFile,
    _context: WebContext,
}

impl Player {
    pub fn new(window: &slint::Window, source: Url) -> Result<Self> {
        let mut page = tempfile::Builder::new()
            .prefix("workshop-video-")
            .suffix(".html")
            .tempfile()?;
        let src = source
            .as_str()
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;");
        write!(
            page,
            r#"<!doctype html><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; media-src file: https: http:; style-src 'unsafe-inline'; script-src 'unsafe-inline'">
<title>Workshop video</title><meta name="referrer" content="no-referrer">
<style>*{{box-sizing:border-box}}body{{margin:0;background:#101214;color:#e8e8e8;font:14px system-ui}}video{{width:100%;height:100vh;display:block}}p{{margin:0;padding:16px}}</style>
<video aria-label="Video" controls autoplay playsinline controlslist="nodownload nofullscreen noremoteplayback" disablepictureinpicture></video>
<p hidden>Workshop could not play this video. The file may be unavailable or its codec unsupported.</p>
<script>const v=document.querySelector('video');v.onerror=()=>{{v.style.display='none';document.querySelector('p').hidden=false}};
function label(state) {{v.setAttribute('aria-label',state+' video, '+v.videoWidth+' by '+v.videoHeight)}}
v.onplaying=()=>label('Playing');v.onpause=()=>label('Paused');v.onended=()=>label('Finished');</script>
<template id="source">{src}</template><script>v.src=document.getElementById('source').content.textContent;</script>"#
        )?;
        page.flush()?;
        let url =
            Url::from_file_path(page.path()).map_err(|_| anyhow::anyhow!("Invalid player path"))?;
        let mut context = WebContext::new(Some(workshop::data_dir()?.join("video-player")));
        let allowed = url.to_string();
        let web = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(url.as_str())
            .with_visible(false)
            .with_focused(false)
            .with_autoplay(true)
            .with_navigation_handler(move |target| target == allowed || target == "about:blank")
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .build_as_child(&window.window_handle())
            .context(
                "Start the embedded video player. Microsoft Edge WebView2 Runtime is required",
            )?;
        Ok(Self {
            web,
            _page: page,
            _context: context,
        })
    }

    pub fn position(&self, x: f32, y: f32, w: f32, h: f32, visible: bool) -> Result<()> {
        if !visible || w < 1. || h < 1. {
            self.web.set_visible(false)?;
            return Ok(());
        }
        self.web.set_bounds(wry::Rect {
            position: LogicalPosition::new(x, y).into(),
            size: LogicalSize::new(w, h).into(),
        })?;
        self.web.set_visible(true)?;
        Ok(())
    }
}
