use crate::discord::{self, DiscordNotifier, DiscordSettings};
use std::sync::{Arc, Mutex};

pub struct DiscordPanel {
    search: String,
    starred_only: bool,
    test_status: String,
}

impl DiscordPanel {
    pub fn new() -> Self {
        Self {
            search: String::new(),
            starred_only: false,
            test_status: String::new(),
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        notifier: &mut DiscordNotifier,
        shared: &Arc<Mutex<crate::app::SharedState>>,
    ) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("💬 Discord Notifications");
            ui.add_space(8.0);

            // Connection card
            ui.group(|ui| {
                ui.label(egui::RichText::new("Connection Setup").strong());
                ui.add_space(4.0);

                // Status indicator
                let (status_color, status_text) = if notifier.is_configured() {
                    (egui::Color32::GREEN, "🟢 Configured & Ready")
                } else if notifier.settings.enabled && (!notifier.settings.bot_token.is_empty() || !notifier.settings.channel_id.is_empty()) {
                    (egui::Color32::YELLOW, "🟡 Partially configured")
                } else {
                    (egui::Color32::GRAY, "⚪ Not configured")
                };
                ui.colored_label(status_color, status_text);
                ui.add_space(4.0);

                // Enable toggle
                ui.checkbox(&mut notifier.settings.enabled, "Enable Discord Notifications")
                    .on_hover_text("Turn notifications on/off");

                // Bot token
                ui.horizontal(|ui| {
                    ui.label("Bot Token:")
                        .on_hover_text("Discord bot token. Get it from Discord Developer Portal > Applications > Your Bot > Token");
                    ui.add(
                        egui::TextEdit::singleline(&mut notifier.settings.bot_token)
                            .password(true)
                            .desired_width(300.0)
                            .hint_text("MTE2NDM3...")
                    );
                });

                // Channel ID
                ui.horizontal(|ui| {
                    ui.label("Channel ID:")
                        .on_hover_text("Discord channel ID where messages will be posted. Enable Developer Mode in Discord > right-click channel > Copy Channel ID");
                    ui.add(
                        egui::TextEdit::singleline(&mut notifier.settings.channel_id)
                            .desired_width(200.0)
                            .hint_text("123456789012345678")
                    );
                });

                // User ID
                ui.horizontal(|ui| {
                    ui.label("User ID:")
                        .on_hover_text("Your Discord user ID to ping. Enable Developer Mode > right-click your name > Copy User ID");
                    ui.add(
                        egui::TextEdit::singleline(&mut notifier.settings.user_id)
                            .desired_width(200.0)
                            .hint_text("987654321098765432")
                    );
                });

                // Ping toggle
                ui.checkbox(&mut notifier.settings.ping_user, "Ping me in every notification")
                    .on_hover_text("Adds @-mention to embed so you get notified");

                ui.add_space(4.0);

                // Test button
                ui.horizontal(|ui| {
                    if ui.button("📨 Send Test").on_hover_text("Post a test message to Discord to verify setup").clicked() {
                        match notifier.send_test() {
                            Ok(_) => self.test_status = "✅ Test sent! Check Discord.".to_string(),
                            Err(e) => self.test_status = format!("❌ Error: {}", e),
                        }
                    }
                    if !self.test_status.is_empty() {
                        let color = if self.test_status.contains("✅") {
                            egui::Color32::GREEN
                        } else {
                            egui::Color32::RED
                        };
                        ui.colored_label(color, &self.test_status);
                    }
                });

                ui.hyperlink_to(
                    "📖 How to set up a Discord bot",
                    "https://discord.com/developers/docs/getting-started",
                );
            });

            ui.add_space(12.0);

            // Search + filter
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Filter notifications…")
                        .desired_width(200.0)
                );
                ui.toggle_value(&mut self.starred_only, "⭐ Starred only")
                    .on_hover_text("Show only starred (essential) notification types");
            });

            ui.add_space(8.0);

            // Bulk actions
            ui.horizontal(|ui| {
                if ui.small_button("✓ Enable all").clicked() {
                    for kind in discord::CATALOG {
                        notifier.settings.enabled_kinds.insert(kind.id.to_string(), true);
                    }
                }
                if ui.small_button("✗ Disable all").clicked() {
                    for kind in discord::CATALOG {
                        notifier.settings.enabled_kinds.insert(kind.id.to_string(), false);
                    }
                }
                if ui.small_button("⭐ Essentials only").clicked() {
                    for kind in discord::CATALOG {
                        notifier.settings.enabled_kinds.insert(kind.id.to_string(), kind.essential);
                    }
                }
            });

            ui.add_space(12.0);

            // Notification kinds by category
            let search_lower = self.search.to_lowercase();
            let categories = discord::categories();
            for cat in categories {
                let kinds = discord::kinds_in(cat);
                let matching_count = kinds.iter()
                    .filter(|k| {
                        let matches_starred = !self.starred_only || discord::is_starred(&notifier.settings, k.id);
                        let matches_search = search_lower.is_empty()
                            || k.label.to_lowercase().contains(&search_lower)
                            || k.desc.to_lowercase().contains(&search_lower);
                        matches_starred && matches_search
                    })
                    .count();

                if matching_count == 0 && !search_lower.is_empty() {
                    continue;
                }

                let cat_header = if search_lower.is_empty() {
                    format!("{} ({})", cat, kinds.len())
                } else {
                    format!("{} ({}/{})", cat, matching_count, kinds.len())
                };

                ui.collapsing(cat_header, |ui| {
                    for kind in kinds {
                        let matches_starred = !self.starred_only || discord::is_starred(&notifier.settings, kind.id);
                        let matches_search = search_lower.is_empty()
                            || kind.label.to_lowercase().contains(&search_lower)
                            || kind.desc.to_lowercase().contains(&search_lower);

                        if !matches_starred || !matches_search {
                            continue;
                        }

                        ui.horizontal(|ui| {
                            // Enabled checkbox
                            let mut enabled = discord::is_enabled(&notifier.settings, kind.id);
                            if ui.checkbox(&mut enabled, "").changed() {
                                notifier.settings.enabled_kinds.insert(kind.id.to_string(), enabled);
                            }

                            // Star button
                            let is_starred = discord::is_starred(&notifier.settings, kind.id);
                            let star_icon = if is_starred { "⭐" } else { "☆" };
                            if ui.small_button(star_icon)
                                .on_hover_text(if is_starred {
                                    "Remove from favorites"
                                } else {
                                    "Add to favorites"
                                })
                                .clicked()
                            {
                                if is_starred {
                                    notifier.settings.starred_kinds.remove(kind.id);
                                } else {
                                    notifier.settings.starred_kinds.insert(kind.id.to_string());
                                }
                            }

                            // Label + description
                            ui.vertical(|ui| {
                                ui.label(format!("{} {}", kind.emoji, kind.label))
                                    .on_hover_text(kind.desc);
                            });

                            // Test button
                            if ui.small_button("T").on_hover_text("Send a test notification of this type").clicked() {
                                self.send_test_kind(notifier, kind.id);
                            }
                        });
                    }
                });
            }

            ui.add_space(16.0);

            // Session summary
            ui.group(|ui| {
                ui.label(egui::RichText::new("📊 Session Summary Report").strong());
                ui.add_space(4.0);
                ui.checkbox(&mut notifier.settings.summary_enabled, "Enable periodic session summary")
                    .on_hover_text("Sends a periodic report of session stats (uptime, frequencies, aircraft, etc.)");
                ui.add(
                    egui::Slider::new(&mut notifier.settings.summary_interval_min, 5..=240)
                        .text("Interval (minutes)")
                );
            });

            ui.add_space(12.0);

            // Save button (for completeness; most edits apply live)
            if ui.button("💾 Save Settings").clicked() {
                if let Ok(mut state) = shared.try_lock() {
                    state.config.discord = notifier.settings.clone();
                    state.config.save();
                }
            }

            ui.colored_label(egui::Color32::GRAY, "Settings are saved to ez_sdr_config.json");
        });
    }

    fn send_test_kind(&mut self, notifier: &mut DiscordNotifier, kind_id: &str) {
        let embed = match kind_id {
            "aircraft_new" => {
                let image = discord::fetch_aircraft_image("ABCDEF");
                discord::embed_aircraft("ABCDEF", "TEST123 ", 51.5, -0.1, 35000, 450, 180, image)
            },
            "scanner_hit" => discord::embed_scanner_hit(145_550_000, -45.5),
            "sat_aos" => discord::embed_sat_aos("ISS", 145_800_000, 62.5),
            "sat_los" => discord::embed_sat_los("ISS"),
            "sat_upcoming" => discord::embed_sat_upcoming("ISS", "13:45:00", "13:58:00", 62.5, 145_800_000),
            "rec_started" => discord::embed_recording_started(137_620_000, "WFM", true, false),
            "rec_stopped" => discord::embed_recording_stopped(137_620_000, "WFM", 120, 50_000_000),
            "rec_error" => discord::embed_recording_error("Disk space low"),
            "strong_signal" => discord::embed_strong_signal(145_550_000, 28.5),
            "source_error" => discord::embed_source_error("Device not found"),
            "task_fired" => discord::embed_task_fired("Test Task", 145_550_000),
            _ => discord::embed_generic(
                "Test Notification",
                &format!("This is a test for: {}", kind_id),
                "✅",
                0x0099FF,
            ),
        };
        if let Err(e) = notifier.send_test() {
            self.test_status = format!("❌ Error: {}", e);
        }
    }
}
