//! 右侧参数面板: 与 unlimited.waifu2x.net 控件一一对应.

use super::*;

impl RepairApp {
    pub(super) fn param_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.running;
        div()
            .w(px(280.))
            .flex_shrink_0()
            .h_full()
            .p_3()
            .gap_2()
            .flex()
            .flex_col()
            .bg(rgb(0xf8fafc))
            .border_l_1()
            .border_color(rgb(0xcbd5e1))
            .id("param_panel")
            .overflow_scroll()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("unlimited:waifu2x"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x64748b))
                    .child("本地 ONNX, 与网站同款模型/参数. 图片不上传."),
            )
            .child(self.choice_row(
                "Backend",
                Backend::ALL.iter().copied().map(|b| {
                    (
                        format!("bk-{}", b.label()),
                        b.label().to_string(),
                        self.settings.backend == b,
                        running,
                        move |this: &mut Self, cx: &mut Context<Self>| {
                            this.settings.backend = b;
                            this.persist(cx);
                        },
                    )
                }),
                cx,
            ))
            .child(self.choice_col(
                "Model",
                ModelId::ALL.iter().copied().map(|m| {
                    (
                        format!("md-{}", m.value()),
                        m.label().to_string(),
                        self.settings.model == m,
                        running,
                        move |this: &mut Self, cx: &mut Context<Self>| {
                            this.settings.model = m;
                            if !m.supports_4x() && this.settings.scale == Scale::X4 {
                                this.settings.scale = Scale::X2;
                            }
                            this.persist(cx);
                        },
                    )
                }),
                cx,
            ))
            .child(self.choice_row(
                "DeNoise",
                NOISE_LEVELS.iter().copied().map(|n| {
                    (
                        format!("nz-{n}"),
                        noise_label(n).to_string(),
                        self.settings.noise == n,
                        running,
                        move |this: &mut Self, cx: &mut Context<Self>| {
                            this.settings.noise = n;
                            this.persist(cx);
                        },
                    )
                }),
                cx,
            ))
            .child(self.choice_row(
                "Upscaling",
                Scale::ALL.iter().copied().filter(|s| {
                    *s != Scale::X4 || self.settings.model.supports_4x()
                }).map(|s| {
                    (
                        format!("sc-{}", s.label()),
                        s.label().to_string(),
                        self.settings.scale == s,
                        running,
                        move |this: &mut Self, cx: &mut Context<Self>| {
                            this.settings.scale = s;
                            this.persist(cx);
                        },
                    )
                }),
                cx,
            ))
            .when(!self.settings.model.supports_4x(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xb45309))
                        .child("no 4x support"),
                )
            })
            .child(self.choice_row(
                "Tile",
                TileSize::ALL.iter().copied().map(|t| {
                    (
                        format!("tl-{}", t.px()),
                        t.px().to_string(),
                        self.settings.tile == t,
                        running,
                        move |this: &mut Self, cx: &mut Context<Self>| {
                            this.settings.tile = t;
                            this.persist(cx);
                        },
                    )
                }),
                cx,
            ))
            .child(
                div()
                    .id("tile-shuffle")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .cursor_pointer()
                    .child(if self.settings.shuffle { "☑" } else { "☐" })
                    .child("Shuffle")
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if this.running {
                                return;
                            }
                            this.settings.shuffle = !this.settings.shuffle;
                            this.persist(cx);
                        }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x64748b))
                    .child(if cfg!(target_os = "macos") {
                        "M 系列请用 400 或 640; 左下角应显示 WebGPU"
                    } else {
                        "256 is recommended"
                    }),
            )
            .child(self.choice_row(
                "TTA",
                TtaLevel::ALL.iter().copied().map(|t| {
                    (
                        format!("tta-{}", t.label()),
                        t.label().to_string(),
                        self.settings.tta == t,
                        running || self.settings.backend != Backend::Cpu,
                        move |this: &mut Self, cx: &mut Context<Self>| {
                            this.settings.tta = t;
                            this.persist(cx);
                        },
                    )
                }),
                cx,
            ))
            .when(self.settings.backend != Backend::Cpu, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x94a3b8))
                        .child("GPU/Auto 时网站会隐藏 TTA"),
                )
            })
            .child(self.choice_row(
                "Alpha Channel",
                AlphaMode::ALL.iter().copied().map(|a| {
                    (
                        format!("al-{}", a.label()),
                        a.label().to_string(),
                        self.settings.alpha == a,
                        running,
                        move |this: &mut Self, cx: &mut Context<Self>| {
                            this.settings.alpha = a;
                            this.persist(cx);
                        },
                    )
                }),
                cx,
            ))
            .child(
                div()
                    .id("binarize")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().text_color(rgb(0x334155)).child("输出"))
                    .child(
                        div()
                            .id("binarize-toggle")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .cursor_pointer()
                            .child(if self.settings.binarize { "☑" } else { "☐" })
                            .child("二值化")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if this.running {
                                        return;
                                    }
                                    this.settings.binarize = !this.settings.binarize;
                                    this.persist(cx);
                                }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x64748b))
                            .child("Otsu 阈值写成 1-bit PNG, 谱面体积会小很多"),
                    ),
            )
            .child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(rgb(0x64748b))
                    .child(format!(
                        "工作集 {} · 可用 {} · 预算 {}",
                        fmt_bytes(self.mem_process),
                        fmt_bytes(self.mem_avail),
                        fmt_bytes(self.mem_budget)
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x64748b))
                    .child(format!(
                        "并发 {} · tile {}",
                        self.workers.max(1),
                        self.live_tile.max(1)
                    )),
            )
    }

    fn choice_row<I, F>(
        &self,
        title: &'static str,
        items: I,
        cx: &mut Context<Self>,
    ) -> impl IntoElement
    where
        I: Iterator<Item = (String, String, bool, bool, F)>,
        F: Fn(&mut Self, &mut Context<Self>) + 'static,
    {
        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_1();
        for (id, label, on, disabled, f) in items {
            row = row.child(self.chip(id, label, on, disabled, f, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(rgb(0x334155)).child(title))
            .child(row)
    }

    fn choice_col<I, F>(
        &self,
        title: &'static str,
        items: I,
        cx: &mut Context<Self>,
    ) -> impl IntoElement
    where
        I: Iterator<Item = (String, String, bool, bool, F)>,
        F: Fn(&mut Self, &mut Context<Self>) + 'static,
    {
        let mut col = div().flex().flex_col().gap_1();
        for (id, label, on, disabled, f) in items {
            col = col.child(self.chip(id, label, on, disabled, f, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(rgb(0x334155)).child(title))
            .child(col)
    }

    fn chip<F: Fn(&mut Self, &mut Context<Self>) + 'static>(
        &self,
        id: String,
        label: String,
        on: bool,
        disabled: bool,
        f: F,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let bg = if on { rgb(0x2563eb) } else { rgb(0xffffff) };
        let fg = if on { rgb(0xffffff) } else { rgb(0x334155) };
        div()
            .id(SharedString::from(id))
            .px_2()
            .py_1()
            .rounded_sm()
            .border_1()
            .border_color(if on { rgb(0x1d4ed8) } else { rgb(0xcbd5e1) })
            .bg(bg)
            .text_color(fg)
            .text_xs()
            .when(!disabled, |d| d.cursor_pointer().hover(|s| s.bg(if on { rgb(0x1d4ed8) } else { rgb(0xe2e8f0) })))
            .when(disabled, |d| d.opacity(0.6))
            .child(label)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if disabled {
                        return;
                    }
                    f(this, cx);
                    cx.notify();
                }),
            )
    }
}
