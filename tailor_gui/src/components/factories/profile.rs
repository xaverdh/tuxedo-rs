use adw::prelude::{ExpanderRowExt, PreferencesRowExt};
use gtk::prelude::{BoxExt, ButtonExt, CheckButtonExt, ObjectExt, WidgetExt};
use once_cell::unsync::Lazy;
use relm4::factory::{DynamicIndex, FactoryComponent, FactorySender, FactoryVecDeque};
use relm4::{adw, factory, gtk, Component, ComponentController, Controller};
use relm4_components::simple_combo_box::SimpleComboBox;
use relm4_icons::icon_name;
use tailor_api::{LedDeviceInfo, LedProfile, ProfileInfo};

use super::profile_item_fan::{ProfileItemFan, ProfileItemFanInit};
use super::profile_item_led::{ProfileItemLed, ProfileItemLedInit};
use crate::components::profiles::ProfilesInput;
use crate::state::{hardware_capabilities, TailorStateMsg, STATE};
use crate::templates;

thread_local! {
    static RADIO_GROUP: Lazy<gtk::CheckButton> = Lazy::new(gtk::CheckButton::default);
}

#[derive(Debug)]
pub struct Profile {
    pub name: String,
    pub info: ProfileInfo,
    pub active: bool,
    pub leds: FactoryVecDeque<ProfileItemLed>,
    pub fans: FactoryVecDeque<ProfileItemFan>,
    pub performance: Controller<SimpleComboBox<String>>,
}

#[derive(Debug)]
pub struct ProfileInit {
    pub name: String,
    pub info: ProfileInfo,
    pub active: bool,
    pub led_profiles: Vec<String>,
    pub fan_profiles: Vec<String>,
}

#[derive(Debug)]
pub enum ProfileInput {
    Enabled,
    UpdateProfile,
}

#[factory(pub)]
impl FactoryComponent for Profile {
    type CommandOutput = ();
    type Init = ProfileInit;
    type Input = ProfileInput;
    type Output = ProfilesInput;
    type ParentInput = ProfilesInput;
    type ParentWidget = adw::PreferencesGroup;

    view! {
        self.leds.widget().clone() -> adw::ExpanderRow {
            set_title: &self.name,
            set_hexpand: true,

            #[chain(build())]
            bind_property: ("expanded", &delete_button, "visible"),

            add_prefix = &gtk::Box {
                set_valign: gtk::Align::Center,

                gtk::CheckButton {
                    #[watch]
                    set_active: self.active,

                    set_group: Some(&RADIO_GROUP.with(|g| (**g).clone())),

                    connect_toggled[sender, index] => move |btn| {
                        if btn.is_active() {
                            sender.input(ProfileInput::Enabled);
                            sender.output(ProfilesInput::Enabled(index.clone()));
                       }
                    },
                },
            },

            add_action = &gtk::Box {
                set_valign: gtk::Align::Center,
                set_margin_end: 2,

                #[name = "delete_button"]
                gtk::Button {
                    set_icon_name: icon_name::CROSS_FILLED,
                    add_css_class: "destructive-action",
                    set_visible: false,
                    #[watch]
                    set_sensitive: !self.active,
                    connect_clicked[sender, index] => move |_| {
                        sender.output(ProfilesInput::Remove(index.clone()));
                    }
                }
            },

            #[template]
            add_row = &templates::ProfileListItem {
                #[template_child]
                image -> gtk::Image {
                    set_icon_name: Some(icon_name::SPEEDOMETER),
                },

                #[template_child]
                label -> gtk::Label {
                    set_label: "performance profile"
                },

                #[template_child]
                row -> gtk::Box {
                    append: self.performance.widget(),
                }
            }
        }
    }

    fn forward_to_parent(output: Self::Output) -> Option<ProfilesInput> {
        Some(output)
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, sender: FactorySender<Self>) -> Self {
        let ProfileInit {
            name,
            mut info,
            active,
            led_profiles,
            fan_profiles,
        } = init;

        let factory_widget = adw::ExpanderRow::new();

        let capabilities = hardware_capabilities().unwrap();

        if info.fans.len() as u8 != capabilities.num_of_fans {
            info.fans
                .resize(capabilities.num_of_fans as usize, "default".to_owned());
        }

        let mut additional_led_profiles = Vec::new();
        for device in &capabilities.led_devices {
            if !info.leds.iter().any(|profile| {
                profile.device_name == device.device_name && profile.function == device.function
            }) {
                additional_led_profiles.push(LedProfile {
                    device_name: device.device_name.clone(),
                    function: device.function.clone(),
                    profile: "default".to_owned(),
                })
            }
        }
        info.leds.extend(additional_led_profiles);

        let mut leds = FactoryVecDeque::new(factory_widget.clone(), sender.input_sender());
        {
            let mut guard = leds.guard();
            for profile in &info.leds {
                let device_info = LedDeviceInfo {
                    device_name: profile.device_name.clone(),
                    function: profile.function.clone(),
                };
                let index = led_profiles
                    .iter()
                    .position(|name| name == &profile.profile)
                    .unwrap_or_default();
                guard.push_back(ProfileItemLedInit {
                    device_info,
                    led_profiles: led_profiles.clone(),
                    index,
                });
            }
        }

        let mut fans = FactoryVecDeque::new(factory_widget, sender.input_sender());
        {
            let mut guard = fans.guard();
            for (idx, profile) in info.fans.iter().enumerate() {
                let index = fan_profiles
                    .iter()
                    .position(|name| name == profile)
                    .unwrap_or_default();
                guard.push_back(ProfileItemFanInit {
                    fan_idx: idx as u8,
                    fan_profiles: fan_profiles.clone(),
                    index,
                });
            }
        }

        let active_index = info.performance_profile.as_ref().and_then(|profile| {
            capabilities
                .performance_profiles
                .iter()
                .position(|name| name == profile)
        });
        let performance = SimpleComboBox::builder()
            .launch(SimpleComboBox {
                variants: capabilities.performance_profiles.clone(),
                active_index,
            })
            .forward(sender.input_sender(), |_| ProfileInput::UpdateProfile);

        Self {
            name,
            info,
            active,
            leds,
            fans,
            performance,
        }
    }

    fn update(&mut self, message: Self::Input, sender: FactorySender<Self>) {
        let name = self.name.clone();

        match message {
            ProfileInput::Enabled => {
                if !self.active {
                    sender.oneshot_command(async move {
                        STATE.emit(TailorStateMsg::SetActiveProfile(name));
                    });
                }
            }
            ProfileInput::UpdateProfile => {
                let leds = self.leds.iter().map(|led| led.get_profile()).collect();

                let fans = self.fans.iter().map(|fan| fan.get_profile_name()).collect();

                let performance_profile = self
                    .performance
                    .state()
                    .get()
                    .model
                    .get_active_elem()
                    .cloned();

                self.info = ProfileInfo {
                    leds,
                    fans,
                    performance_profile,
                };

                let profile = self.info.clone();

                sender.oneshot_command(async move {
                    STATE.emit(TailorStateMsg::AddProfile { name, profile });
                });
            }
        }
    }
}
