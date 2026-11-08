//! The "App updates" dialog and header chip that every app shows around
//! [`app_update::Updater`].
//!
//! What the dialog says, in what order, and what each choice does live here,
//! so the apps cannot drift apart. Each app keeps its own look and its own
//! container. It implements [`Look`] for its colours, spacing, text styles,
//! icons, and buttons. It draws [`body`] and then [`actions`] in its own window
//! or modal, titled [`TITLE`], and passes what the user chose to [`apply`]. Its
//! header shows [`chip_text`] in its own chip widget, with [`chip_hover_text`],
//! and opens the dialog when the chip is clicked.
//!
//! [`body`] keeps the release notes in a scroll area of their own, so the check
//! settings and the buttons stay in view however long the notes run. A
//! container that already scrolls its whole body turns that off through
//! [`Look::notes_height`].
//!
//! ```no_run
//! use app_update::{Settings, Updater};
//!
//! struct Look;
//! impl app_update_ui::Look for Look {}
//!
//! fn window(
//!     ctx: &egui::Context,
//!     open: &mut bool,
//!     updater: &mut Updater,
//!     settings: &mut Settings,
//!     busy: bool,
//! ) {
//!     let mut chosen = None;
//!     egui::Window::new(app_update_ui::TITLE)
//!         .open(open)
//!         .collapsible(false)
//!         .resizable(false)
//!         .show(ctx, |ui| {
//!             ui.set_width(520.0);
//!             chosen = app_update_ui::body(ui, &Look, updater, settings);
//!             let busy = busy.then_some("Wait for the backup in progress to finish");
//!             chosen = app_update_ui::actions(ui, &Look, updater, busy).or(chosen);
//!         });
//!     if let Some(action) = chosen
//!         && app_update_ui::apply(action, ctx, updater, settings)
//!     {
//!         *open = false;
//!     }
//! }
//! ```

use app_update::{CheckPeriod, NoteBlock, Release, Settings, Update, Updater};
use egui::text::LayoutJob;
use egui::{
    Align, Button, Color32, ComboBox, Context, FontSelection, Label, Layout, ProgressBar, RichText,
    ScrollArea, Ui, ViewportCommand, Visuals, WidgetText,
};

/// The dialog's title, for the app's window or modal.
pub const TITLE: &str = "App updates";

/// Roughly how tall the dialog is without the release notes: the container's
/// title and margins, the status, the check settings, and the buttons.
const AROUND_NOTES: f32 = 380.0;

/// What the user chose in the dialog. Pass it to [`apply`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    /// Check GitHub for a newer release now.
    CheckNow,
    /// Download and verify the latest release.
    Download,
    /// Swap the downloaded release in and restart into it.
    Restart,
    /// Stop pointing out the latest release until a newer one is out.
    Skip,
    /// Stop pointing out the latest release until the app starts again.
    Later,
}

/// What an icon beside a label or on a button stands for. An app picks a
/// glyph for each one it wants in [`Look::glyph`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Icon {
    /// The "Check now" button.
    CheckNow,
    /// The "Download update" button.
    Download,
    /// The "Restart to update" button.
    Restart,
    /// The "Skip this version" button.
    Skip,
    /// The "Not now" button.
    Later,
    /// The line saying the app is up to date, or that an update is ready.
    Success,
    /// A failed check, download, or install.
    Warning,
    /// The links to the release page and to every release.
    Link,
}

/// How one app styles the dialog. Every method has a default built on egui's
/// own style, so an app overrides only what its theme sets.
pub trait Look {
    /// Secondary text: the running version, a skipped release, the release
    /// dates, the bullets, and the fine print.
    fn muted(&self, visuals: &Visuals) -> Color32 {
        visuals.weak_text_color()
    }

    /// Good news: the app is up to date, or an update is ready.
    fn success(&self, visuals: &Visuals) -> Color32 {
        if visuals.dark_mode {
            Color32::from_rgb(80, 160, 100)
        } else {
            Color32::from_rgb(30, 120, 60)
        }
    }

    /// A failed check, download, or install.
    fn warning(&self, visuals: &Visuals) -> Color32 {
        visuals.warn_fg_color
    }

    /// The space between a line and the detail under it.
    fn space_xs(&self) -> f32 {
        4.0
    }

    /// The space before an error, and between releases in the notes.
    fn space_sm(&self) -> f32 {
        8.0
    }

    /// The space between sections.
    fn space_md(&self) -> f32 {
        12.0
    }

    /// "Version X is available", the dialog's headline.
    fn heading(&self, text: RichText) -> RichText {
        text.heading()
    }

    /// A section title: "What's new" and "Checking".
    fn section(&self, text: RichText) -> RichText {
        self.bold(text)
    }

    /// A release's version above its notes, and a heading inside the notes.
    fn bold(&self, text: RichText) -> RichText {
        text.strong()
    }

    /// A release's date and the line saying when the last check ran. The
    /// text arrives coloured [`Look::muted`].
    fn fine_print(&self, text: RichText) -> RichText {
        text.small()
    }

    /// The glyph to show for `icon`, or `None` for none. [`Look::icon_text`]
    /// puts it before the label.
    fn glyph(&self, icon: Icon) -> Option<&'static str> {
        let _ = icon;
        None
    }

    /// A label led by `icon`'s glyph, in `color` or, given `None`, in the
    /// colour of the widget it goes on. The default joins the glyph and the
    /// label with two spaces, for a theme that falls back to an icon font.
    fn icon_text(&self, icon: Icon, text: &str, color: Option<Color32>) -> WidgetText {
        let text = match self.glyph(icon) {
            Some(glyph) => RichText::new(format!("{glyph}  {text}")),
            None => RichText::new(text),
        };
        match color {
            Some(color) => text.color(color).into(),
            None => text.into(),
        }
    }

    /// A secondary button: "Check now", "Skip this version", and "Not now".
    fn button(&self, icon: Icon, text: &str) -> Button<'static> {
        Button::new(self.icon_text(icon, text, None))
    }

    /// The button that moves the update on: "Download update" or "Restart to
    /// update".
    fn primary_button(&self, icon: Icon, text: &str) -> Button<'static> {
        self.button(icon, text)
    }

    /// Shows the last check, download, or install failure.
    fn show_error(&self, ui: &mut Ui, message: &str) {
        let color = self.warning(ui.visuals());
        ui.add(Label::new(self.icon_text(Icon::Warning, message, Some(color))).wrap());
    }

    /// The most the release notes may take before they scroll, or `None` to
    /// lay them out in full, for a container that scrolls its whole body. The
    /// default leaves the rest of the dialog on screen.
    fn notes_height(&self, ui: &Ui) -> Option<f32> {
        Some((ui.ctx().content_rect().height() - AROUND_NOTES).clamp(120.0, 400.0))
    }
}

/// The chip's text while the header should point out a new release:
/// "Update X available", or "Restart for X" once it is downloaded. `None`
/// when there is nothing to point out, so the app shows no chip.
#[must_use]
pub fn chip_text(updater: &Updater, settings: &Settings) -> Option<String> {
    let release = updater.notification(settings)?;
    Some(if updater.staged() == Some(release.version) {
        format!("Restart for {}", release.version)
    } else {
        format!("Update {} available", release.version)
    })
}

/// The chip's hover text.
#[must_use]
pub fn chip_hover_text(updater: &Updater) -> String {
    format!(
        "A new version of {} is out. Click to see what changed.",
        updater.app().name
    )
}

/// The dialog above its buttons: where the running version stands, the notes
/// of every newer release, the last failure, and how often to check. Writes
/// the chosen check period into `settings`, and returns
/// [`Action::CheckNow`] when "Check now" was clicked.
#[must_use = "the action does nothing until it is passed to `apply`"]
pub fn body(
    ui: &mut Ui,
    look: &(impl Look + ?Sized),
    updater: &Updater,
    settings: &mut Settings,
) -> Option<Action> {
    let current = updater.current().map_or_else(
        || updater.app().version.to_owned(),
        |version| version.to_string(),
    );
    let muted = look.muted(ui.visuals());
    if let Some(update) = updater.update() {
        available(ui, look, updater, settings, update, &current);
    } else if updater.is_checking() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Checking GitHub for a new version…");
        });
    } else if updater.is_up_to_date() {
        let success = look.success(ui.visuals());
        ui.label(look.icon_text(
            Icon::Success,
            &format!("Version {current} is the latest version."),
            Some(success),
        ));
    } else {
        ui.label(RichText::new(format!("You have version {current}.")).color(muted));
    }
    if let Some(error) = updater.error() {
        ui.add_space(look.space_sm());
        look.show_error(ui, error);
    }
    schedule(ui, look, updater, settings)
}

/// The newest version, where its install stands, and the notes of every
/// release since the running one.
fn available(
    ui: &mut Ui,
    look: &(impl Look + ?Sized),
    updater: &Updater,
    settings: &Settings,
    update: &Update,
    current: &str,
) {
    let name = updater.app().name;
    let muted = look.muted(ui.visuals());
    let latest = update.latest();
    ui.label(look.heading(RichText::new(format!(
        "Version {} is available",
        latest.version
    ))));
    let released = if latest.date.is_empty() {
        String::new()
    } else {
        format!(" It was released on {}.", latest.date)
    };
    ui.label(RichText::new(format!("You have version {current}.{released}")).color(muted));
    if updater.staged() == Some(latest.version) {
        ui.add_space(look.space_xs());
        let success = look.success(ui.visuals());
        ui.label(look.icon_text(
            Icon::Success,
            &format!("Downloaded and verified. Restart {name} to finish."),
            Some(success),
        ));
    } else if settings.skipped.as_deref() == Some(latest.version.to_string().as_str()) {
        ui.add_space(look.space_xs());
        ui.label(
            RichText::new(format!(
                "You skipped this version, so {name} does not point it out."
            ))
            .color(muted),
        );
    }
    if !updater.can_install() {
        ui.add_space(look.space_xs());
        ui.label(
            RichText::new(format!(
                "This release has no build that {name} can install by itself. \
                 Download it from the release page instead."
            ))
            .color(muted),
        );
        ui.hyperlink_to(
            look.icon_text(Icon::Link, "Release page", None),
            &latest.page_url,
        );
    }

    ui.add_space(look.space_md());
    ui.label(look.section(RichText::new("What's new")));
    ui.add_space(look.space_xs());
    match look.notes_height(ui) {
        Some(height) => {
            ScrollArea::vertical()
                .id_salt("app-update-notes")
                .max_height(height)
                // A centred container sizes itself with only the space below
                // its centre, so without this, long notes would settle at
                // half the height they may take.
                .min_scrolled_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| releases(ui, look, update));
        }
        None => releases(ui, look, update),
    }
    ui.add_space(look.space_xs());
    ui.hyperlink_to(
        look.icon_text(Icon::Link, "All releases", None),
        updater.app().releases_page(),
    );
}

/// Every newer release, newest first: its version, its date, and its notes.
fn releases(ui: &mut Ui, look: &(impl Look + ?Sized), update: &Update) {
    for (index, release) in update.releases().enumerate() {
        if index > 0 {
            ui.add_space(look.space_sm());
        }
        release_heading(ui, look, release);
        notes(ui, look, &release.notes);
    }
}

/// A release's version and date as one label. A row of two labels would be a
/// button tall, and stand further from the notes than a line of text.
fn release_heading(ui: &mut Ui, look: &(impl Look + ?Sized), release: &Release) {
    let job = {
        let style = ui.style();
        let mut job = LayoutJob::default();
        look.bold(RichText::new(release.version.to_string()))
            .append_to(&mut job, style, FontSelection::Default, Align::Center);
        if !release.date.is_empty() {
            let first = job.sections.len();
            look.fine_print(RichText::new(&release.date).color(look.muted(&style.visuals)))
                .append_to(&mut job, style, FontSelection::Default, Align::Center);
            if let Some(section) = job.sections.get_mut(first) {
                section.leading_space = style.spacing.item_spacing.x;
            }
        }
        job
    };
    ui.label(job);
}

/// One release's notes, laid out from the Markdown subset `CHANGELOG.md` uses.
fn notes(ui: &mut Ui, look: &(impl Look + ?Sized), markdown: &str) {
    let muted = look.muted(ui.visuals());
    let blocks = app_update::parse_notes(markdown);
    if blocks.is_empty() {
        ui.label(RichText::new("No notes for this release.").color(muted));
    }
    for block in blocks {
        match block {
            NoteBlock::Heading(text) => {
                ui.label(look.bold(RichText::new(text)));
            }
            NoteBlock::Bullet(text) => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new("•").color(muted));
                    ui.add(Label::new(text).wrap());
                });
            }
            NoteBlock::Paragraph(text) => {
                ui.add(Label::new(text).wrap());
            }
        }
    }
}

/// How often to check, a way to check now, and when the last check ran.
fn schedule(
    ui: &mut Ui,
    look: &(impl Look + ?Sized),
    updater: &Updater,
    settings: &mut Settings,
) -> Option<Action> {
    ui.add_space(look.space_md());
    ui.separator();
    ui.label(look.section(RichText::new("Checking")));
    ui.add_space(look.space_xs());
    let mut check = false;
    ui.horizontal(|ui| {
        ui.label("Check for new versions");
        ComboBox::from_id_salt("app-update-period")
            .selected_text(settings.period.label())
            .show_ui(ui, |ui| {
                for option in CheckPeriod::ALL {
                    ui.selectable_value(&mut settings.period, option, option.label());
                }
            });
        check = ui
            .add_enabled(!updater.is_busy(), look.button(Icon::CheckNow, "Check now"))
            .clicked();
    });
    let last = settings.last_check.map_or_else(
        || "Never checked".to_owned(),
        |then| {
            format!(
                "Last checked {}",
                app_update::describe_age(then, app_update::now())
            )
        },
    );
    let muted = look.muted(ui.visuals());
    ui.label(
        look.fine_print(
            RichText::new(format!(
                "{last}. A check asks GitHub for this app's releases and sends nothing else."
            ))
            .color(muted),
        ),
    );
    check.then_some(Action::CheckNow)
}

/// The dialog's bottom row while an update is out: a progress bar while it
/// downloads, otherwise the buttons that decide what happens to it. Draws
/// nothing when there is no update.
///
/// `busy` is `None` while the app is idle. While the app runs work that
/// closing it would cut short, `busy` is the hover text for the disabled
/// restart button, such as "Wait for the backup in progress to finish".
#[must_use = "the action does nothing until it is passed to `apply`"]
pub fn actions(
    ui: &mut Ui,
    look: &(impl Look + ?Sized),
    updater: &Updater,
    busy: Option<&str>,
) -> Option<Action> {
    let update = updater.update()?;
    ui.add_space(look.space_md());
    if let Some((received, total)) = updater.progress() {
        let percent = percent(received, total);
        ui.add(ProgressBar::new(f32::from(percent) / 100.0).text(format!(
            "Downloading {}… {percent}%",
            update.latest().version
        )));
        return None;
    }
    let name = updater.app().name;
    let mut action = None;
    // Right to left, so the button that moves the update on sits at the end.
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if updater.staged().is_some() {
                let restart = ui
                    .add_enabled(
                        busy.is_none(),
                        look.primary_button(Icon::Restart, "Restart to update"),
                    )
                    .on_hover_text(format!("Close {name} and open the new version"));
                let restart = match busy {
                    Some(hint) => restart.on_disabled_hover_text(hint),
                    None => restart,
                };
                if restart.clicked() {
                    action = Some(Action::Restart);
                }
            } else if updater.can_install()
                && ui
                    .add_enabled(
                        !updater.is_busy(),
                        look.primary_button(Icon::Download, "Download update"),
                    )
                    .on_hover_text(
                        "Download the new version and check it against its published SHA-256",
                    )
                    .clicked()
            {
                action = Some(Action::Download);
            }
            if ui
                .add(look.button(Icon::Skip, "Skip this version"))
                .on_hover_text("Stop pointing out this version. A newer one shows up again.")
                .clicked()
            {
                action = Some(Action::Skip);
            }
            if ui
                .add(look.button(Icon::Later, "Not now"))
                .on_hover_text(format!("Hide the reminder until {name} starts again"))
                .clicked()
            {
                action = Some(Action::Later);
            }
        });
    });
    action
}

/// Carries out what the user chose. Returns whether the dialog should close.
///
/// A restart that swaps the new version in closes the app's window, and the
/// app starts the new version once its event loop returns. A restart that
/// fails keeps the dialog open, showing why.
#[must_use = "true means the dialog should close"]
pub fn apply(
    action: Action,
    ctx: &Context,
    updater: &mut Updater,
    settings: &mut Settings,
) -> bool {
    match action {
        Action::CheckNow => {
            updater.check_now();
            false
        }
        Action::Download => {
            updater.download();
            false
        }
        Action::Restart => {
            if updater.install_and_restart().is_ok() {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            false
        }
        Action::Skip => {
            updater.skip(settings);
            true
        }
        Action::Later => {
            updater.dismiss();
            true
        }
    }
}

/// How much of a download has arrived, in whole percent, rounded down so 100
/// means complete. An unknown or zero total reads as 0, and more bytes than
/// expected as 100.
fn percent(received: u64, total: Option<u64>) -> u8 {
    let percent = total
        .filter(|total| *total > 0)
        .map_or(0, |total| u128::from(received) * 100 / u128::from(total));
    u8::try_from(percent.min(100)).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The progress bar divides by a total the server reports, so a missing
    // or zero total must not panic, and huge or overshooting counts must not
    // wrap or run past the end of the bar.
    #[test]
    fn download_percent_stays_within_the_bar() {
        assert_eq!(percent(0, None), 0);
        assert_eq!(percent(500, None), 0);
        assert_eq!(percent(500, Some(0)), 0);
        assert_eq!(percent(50, Some(200)), 25);
        assert_eq!(percent(199, Some(200)), 99);
        assert_eq!(percent(200, Some(200)), 100);
        assert_eq!(percent(300, Some(200)), 100);
        assert_eq!(percent(u64::MAX / 2, Some(u64::MAX)), 49);
        assert_eq!(percent(u64::MAX, Some(u64::MAX)), 100);
    }
}
