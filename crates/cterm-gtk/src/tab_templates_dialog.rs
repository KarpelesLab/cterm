//! Tab Templates Dialog for GTK4
//!
//! Provides a window for managing tab templates (sticky tabs).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, CheckButton, ColorButton, ComboBoxText, Dialog, Entry, Grid,
    Label, Notebook, Orientation, ResponseType, ScrolledWindow, Window,
};

use cterm_app::config::{
    load_sticky_tabs, save_sticky_tabs, DockerMode, DockerTabConfig, SshPortForward, SshTabConfig,
    StickyTabConfig,
};

/// Widgets for the tab templates dialog
struct TemplateWidgets {
    // Template selector
    template_combo: ComboBoxText,
    // General tab
    name_entry: Entry,
    command_entry: Entry,
    args_entry: Entry,
    path_entry: Entry,
    color_button: ColorButton,
    theme_entry: Entry,
    unique_check: CheckButton,
    auto_start_check: CheckButton,
    keep_open_check: CheckButton,
    // Docker tab
    docker_mode_combo: ComboBoxText,
    docker_container_entry: Entry,
    docker_image_entry: Entry,
    docker_shell_entry: Entry,
    docker_auto_remove_check: CheckButton,
    docker_project_dir_entry: Entry,
    #[allow(dead_code)]
    docker_status_label: Label,
    // Container row for visibility control
    docker_container_row: GtkBox,
    docker_image_row: GtkBox,
    docker_shell_row: GtkBox,
    docker_project_row: GtkBox,
    // SSH tab
    ssh_enabled_check: CheckButton,
    ssh_host_entry: Entry,
    ssh_port_entry: Entry,
    ssh_username_entry: Entry,
    ssh_identity_entry: Entry,
    ssh_jump_host_entry: Entry,
    ssh_local_forward_entry: Entry,
    ssh_remote_command_entry: Entry,
    ssh_x11_check: CheckButton,
    ssh_agent_check: CheckButton,
}

/// Show the tab templates dialog
pub fn show_tab_templates_dialog(parent: &impl IsA<Window>, on_save: impl Fn() + 'static) {
    let templates = load_sticky_tabs().unwrap_or_default();
    let templates = Rc::new(RefCell::new(templates));

    let dialog = Dialog::builder()
        .title("Tab Templates")
        .transient_for(parent)
        .modal(true)
        .default_width(550)
        .default_height(520)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Save", ResponseType::Ok);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Template selector row
    let selector_row = GtkBox::new(Orientation::Horizontal, 8);

    let selector_label = Label::new(Some("Template:"));
    selector_row.append(&selector_label);

    let template_combo = ComboBoxText::new();
    template_combo.set_hexpand(true);
    for template in templates.borrow().iter() {
        template_combo.append_text(&template.name);
    }
    if !templates.borrow().is_empty() {
        template_combo.set_active(Some(0));
    }
    selector_row.append(&template_combo);

    let add_button = Button::with_label("+");
    add_button.set_tooltip_text(Some("Add new template"));
    selector_row.append(&add_button);

    let remove_button = Button::with_label("-");
    remove_button.set_tooltip_text(Some("Remove template"));
    selector_row.append(&remove_button);

    // Presets dropdown
    let presets_combo = ComboBoxText::new();
    presets_combo.append_text("Add Preset...");
    presets_combo.append_text("Claude Code");
    presets_combo.append_text("Claude Container");
    presets_combo.append_text("Ubuntu Container");
    presets_combo.append_text("Alpine Container");
    presets_combo.append_text("Node.js Container");
    presets_combo.append_text("Python Container");
    presets_combo.append_text("SSH Connection");
    presets_combo.append_text("SSH with Agent Forwarding");
    presets_combo.set_active(Some(0));
    selector_row.append(&presets_combo);

    content.append(&selector_row);

    // Create notebook for tabs
    let notebook = Notebook::new();
    notebook.set_vexpand(true);
    content.append(&notebook);

    // Create widgets
    let widgets = Rc::new(create_widgets(&notebook));

    // Load first template if available
    if !templates.borrow().is_empty() {
        load_template_into_widgets(&widgets, &templates.borrow()[0]);
    }

    // Connect template selector
    {
        let widgets = Rc::clone(&widgets);
        let templates = Rc::clone(&templates);
        template_combo.connect_changed(move |combo| {
            if let Some(index) = combo.active() {
                let templates = templates.borrow();
                if let Some(template) = templates.get(index as usize) {
                    load_template_into_widgets(&widgets, template);
                }
            }
        });
    }

    // Connect add button
    {
        let templates = Rc::clone(&templates);
        let widgets = Rc::clone(&widgets);
        let combo = widgets.template_combo.clone();
        add_button.connect_clicked(move |_| {
            let new_template = StickyTabConfig {
                name: "New Template".into(),
                ..Default::default()
            };
            templates.borrow_mut().push(new_template.clone());
            combo.append_text(&new_template.name);
            let new_index = templates.borrow().len() - 1;
            combo.set_active(Some(new_index as u32));
            load_template_into_widgets(&widgets, &new_template);
        });
    }

    // Connect remove button
    {
        let templates = Rc::clone(&templates);
        let widgets = Rc::clone(&widgets);
        let combo = widgets.template_combo.clone();
        remove_button.connect_clicked(move |_| {
            if let Some(index) = combo.active() {
                let index = index as usize;
                let mut templates = templates.borrow_mut();
                if templates.len() > 1 && index < templates.len() {
                    templates.remove(index);
                    combo.remove(index as i32);
                    let new_index = if index > 0 { index - 1 } else { 0 };
                    combo.set_active(Some(new_index as u32));
                    drop(templates);
                }
            }
        });
    }

    // Connect presets dropdown
    {
        let templates = Rc::clone(&templates);
        let widgets = Rc::clone(&widgets);
        let template_combo = widgets.template_combo.clone();
        presets_combo.connect_changed(move |combo| {
            if let Some(index) = combo.active() {
                if index == 0 {
                    return; // "Add Preset..." label
                }
                let new_template = create_preset_template(index as usize);
                if let Some(template) = new_template {
                    templates.borrow_mut().push(template.clone());
                    template_combo.append_text(&template.name);
                    let new_index = templates.borrow().len() - 1;
                    template_combo.set_active(Some(new_index as u32));
                    load_template_into_widgets(&widgets, &template);
                }
                combo.set_active(Some(0)); // Reset to label
            }
        });
    }

    // Connect field changes to save to template
    connect_field_signals(&widgets, Rc::clone(&templates));

    // Handle dialog response
    let templates_for_save = Rc::clone(&templates);
    let widgets_for_save = Rc::clone(&widgets);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Ok {
            // Save current fields to template before saving
            if let Some(index) = widgets_for_save.template_combo.active() {
                let mut templates = templates_for_save.borrow_mut();
                if let Some(template) = templates.get_mut(index as usize) {
                    save_widgets_to_template(&widgets_for_save, template);
                }
            }

            // Save to disk
            let templates = templates_for_save.borrow();
            if let Err(e) = save_sticky_tabs(&templates) {
                log::error!("Failed to save tab templates: {}", e);
            } else {
                log::info!("Tab templates saved ({} templates)", templates.len());
            }
            on_save();
        }
        dialog.close();
    });

    dialog.present();
}

fn create_widgets(notebook: &Notebook) -> TemplateWidgets {
    // General tab
    let (
        general_page,
        name_entry,
        command_entry,
        args_entry,
        path_entry,
        color_button,
        theme_entry,
        unique_check,
        auto_start_check,
        keep_open_check,
    ) = create_general_tab();
    notebook.append_page(&general_page, Some(&Label::new(Some("General"))));

    // Docker tab
    let (
        docker_page,
        docker_mode_combo,
        docker_container_entry,
        docker_image_entry,
        docker_shell_entry,
        docker_auto_remove_check,
        docker_project_dir_entry,
        docker_status_label,
        docker_container_row,
        docker_image_row,
        docker_shell_row,
        docker_project_row,
    ) = create_docker_tab();
    notebook.append_page(&docker_page, Some(&Label::new(Some("Docker"))));

    // SSH tab
    let (
        ssh_page,
        ssh_enabled_check,
        ssh_host_entry,
        ssh_port_entry,
        ssh_username_entry,
        ssh_identity_entry,
        ssh_jump_host_entry,
        ssh_local_forward_entry,
        ssh_remote_command_entry,
        ssh_x11_check,
        ssh_agent_check,
    ) = create_ssh_tab();
    notebook.append_page(&ssh_page, Some(&Label::new(Some("Remote/SSH"))));

    // We need to get the template_combo from the dialog, but we'll set a placeholder
    // This is a bit awkward but we'll wire it up after creation
    let template_combo = ComboBoxText::new();

    TemplateWidgets {
        template_combo,
        name_entry,
        command_entry,
        args_entry,
        path_entry,
        color_button,
        theme_entry,
        unique_check,
        auto_start_check,
        keep_open_check,
        docker_mode_combo,
        docker_container_entry,
        docker_image_entry,
        docker_shell_entry,
        docker_auto_remove_check,
        docker_project_dir_entry,
        docker_status_label,
        docker_container_row,
        docker_image_row,
        docker_shell_row,
        docker_project_row,
        ssh_enabled_check,
        ssh_host_entry,
        ssh_port_entry,
        ssh_username_entry,
        ssh_identity_entry,
        ssh_jump_host_entry,
        ssh_local_forward_entry,
        ssh_remote_command_entry,
        ssh_x11_check,
        ssh_agent_check,
    }
}

fn create_general_tab() -> (
    ScrolledWindow,
    Entry,
    Entry,
    Entry,
    Entry,
    ColorButton,
    Entry,
    CheckButton,
    CheckButton,
    CheckButton,
) {
    let scroll = ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let page = GtkBox::new(Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);

    let mut row = 0;

    // Name
    let name_label = Label::new(Some("Name:"));
    name_label.set_halign(Align::End);
    grid.attach(&name_label, 0, row, 1, 1);
    let name_entry = Entry::new();
    name_entry.set_hexpand(true);
    grid.attach(&name_entry, 1, row, 1, 1);
    row += 1;

    // Command
    let command_label = Label::new(Some("Command:"));
    command_label.set_halign(Align::End);
    grid.attach(&command_label, 0, row, 1, 1);
    let command_entry = Entry::new();
    command_entry.set_placeholder_text(Some("(default shell)"));
    grid.attach(&command_entry, 1, row, 1, 1);
    row += 1;

    // Arguments
    let args_label = Label::new(Some("Arguments:"));
    args_label.set_halign(Align::End);
    grid.attach(&args_label, 0, row, 1, 1);
    let args_entry = Entry::new();
    args_entry.set_placeholder_text(Some("space-separated"));
    grid.attach(&args_entry, 1, row, 1, 1);
    row += 1;

    // Working directory
    let path_label = Label::new(Some("Working Dir:"));
    path_label.set_halign(Align::End);
    grid.attach(&path_label, 0, row, 1, 1);
    let path_entry = Entry::new();
    grid.attach(&path_entry, 1, row, 1, 1);
    row += 1;

    // Color
    let color_label = Label::new(Some("Tab Color:"));
    color_label.set_halign(Align::End);
    grid.attach(&color_label, 0, row, 1, 1);
    let color_box = GtkBox::new(Orientation::Horizontal, 8);
    let color_button = ColorButton::new();
    color_box.append(&color_button);
    let clear_color_button = Button::with_label("Clear");
    let color_button_clone = color_button.clone();
    clear_color_button.connect_clicked(move |_| {
        color_button_clone.set_rgba(&gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    });
    color_box.append(&clear_color_button);
    grid.attach(&color_box, 1, row, 1, 1);
    row += 1;

    // Theme
    let theme_label = Label::new(Some("Theme:"));
    theme_label.set_halign(Align::End);
    grid.attach(&theme_label, 0, row, 1, 1);
    let theme_entry = Entry::new();
    theme_entry.set_placeholder_text(Some("(default)"));
    grid.attach(&theme_entry, 1, row, 1, 1);

    page.append(&grid);

    // Checkboxes
    let check_box = GtkBox::new(Orientation::Vertical, 4);
    check_box.set_margin_top(12);

    let unique_check = CheckButton::with_label("Unique (only one instance allowed)");
    check_box.append(&unique_check);

    let auto_start_check = CheckButton::with_label("Auto-start on launch");
    check_box.append(&auto_start_check);

    let keep_open_check = CheckButton::with_label("Keep tab open after exit");
    check_box.append(&keep_open_check);

    page.append(&check_box);

    scroll.set_child(Some(&page));

    (
        scroll,
        name_entry,
        command_entry,
        args_entry,
        path_entry,
        color_button,
        theme_entry,
        unique_check,
        auto_start_check,
        keep_open_check,
    )
}

#[allow(clippy::type_complexity)]
fn create_docker_tab() -> (
    ScrolledWindow,
    ComboBoxText,
    Entry,
    Entry,
    Entry,
    CheckButton,
    Entry,
    Label,
    GtkBox,
    GtkBox,
    GtkBox,
    GtkBox,
) {
    let scroll = ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let page = GtkBox::new(Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    // Mode dropdown
    let mode_row = GtkBox::new(Orientation::Horizontal, 12);
    let mode_label = Label::new(Some("Mode:"));
    mode_label.set_halign(Align::End);
    mode_label.set_width_request(100);
    mode_row.append(&mode_label);

    let mode_combo = ComboBoxText::new();
    mode_combo.append_text("None (Regular Tab)");
    mode_combo.append_text("Exec (Connect to Container)");
    mode_combo.append_text("Run (Start Container)");
    mode_combo.append_text("DevContainer (With Mounts)");
    mode_combo.set_active(Some(0));
    mode_combo.set_hexpand(true);
    mode_row.append(&mode_combo);
    page.append(&mode_row);

    // Container field (Exec mode)
    let container_row = GtkBox::new(Orientation::Horizontal, 12);
    let container_label = Label::new(Some("Container:"));
    container_label.set_halign(Align::End);
    container_label.set_width_request(100);
    container_row.append(&container_label);
    let container_entry = Entry::new();
    container_entry.set_hexpand(true);
    container_entry.set_placeholder_text(Some("container name or ID"));
    container_row.append(&container_entry);
    page.append(&container_row);

    // Image field (Run/DevContainer mode)
    let image_row = GtkBox::new(Orientation::Horizontal, 12);
    let image_label = Label::new(Some("Image:"));
    image_label.set_halign(Align::End);
    image_label.set_width_request(100);
    image_row.append(&image_label);
    let image_entry = Entry::new();
    image_entry.set_hexpand(true);
    image_entry.set_placeholder_text(Some("image:tag"));
    image_row.append(&image_entry);
    page.append(&image_row);

    // Shell field
    let shell_row = GtkBox::new(Orientation::Horizontal, 12);
    let shell_label = Label::new(Some("Shell:"));
    shell_label.set_halign(Align::End);
    shell_label.set_width_request(100);
    shell_row.append(&shell_label);
    let shell_entry = Entry::new();
    shell_entry.set_hexpand(true);
    shell_entry.set_placeholder_text(Some("/bin/sh"));
    shell_row.append(&shell_entry);
    page.append(&shell_row);

    // Auto-remove checkbox
    let auto_remove_check = CheckButton::with_label("Auto-remove container on exit");
    auto_remove_check.set_active(true);
    auto_remove_check.set_margin_start(112);
    page.append(&auto_remove_check);

    // Project directory field (DevContainer)
    let project_row = GtkBox::new(Orientation::Horizontal, 12);
    let project_label = Label::new(Some("Project Dir:"));
    project_label.set_halign(Align::End);
    project_label.set_width_request(100);
    project_row.append(&project_label);
    let project_entry = Entry::new();
    project_entry.set_hexpand(true);
    project_entry.set_placeholder_text(Some("path to project"));
    project_row.append(&project_entry);
    page.append(&project_row);

    // Status label - shows Docker availability status
    let status_label = Label::new(None);
    status_label.set_halign(Align::Start);
    status_label.set_margin_start(112);
    status_label.set_visible(false);
    page.append(&status_label);

    // Check Docker status and update label
    fn update_docker_status(label: &Label, show: bool) {
        if !show {
            label.set_visible(false);
            return;
        }

        match cterm_app::docker::check_docker_available() {
            Ok(()) => {
                // Docker is available, show container/image count
                // list_containers() returns running containers from `docker ps`
                let running = cterm_app::docker::list_containers()
                    .unwrap_or_default()
                    .len();
                let images = cterm_app::docker::list_images().unwrap_or_default().len();
                label.set_text(&format!(
                    "✓ Docker: {} running containers, {} images",
                    running, images
                ));
                label.remove_css_class("error");
                label.add_css_class("success");
            }
            Err(e) => {
                label.set_text(&format!("✗ Docker: {}", e));
                label.remove_css_class("success");
                label.add_css_class("error");
            }
        }
        label.set_visible(true);
    }

    // Connect mode combo to visibility updates
    let container_row_clone = container_row.clone();
    let image_row_clone = image_row.clone();
    let shell_row_clone = shell_row.clone();
    let project_row_clone = project_row.clone();
    let auto_remove_check_clone = auto_remove_check.clone();
    let status_label_clone = status_label.clone();
    mode_combo.connect_changed(move |combo| {
        let mode_index = combo.active().unwrap_or(0);
        let is_docker = mode_index > 0;
        let is_exec = mode_index == 1;
        let is_run_or_dev = mode_index >= 2;
        let is_devcontainer = mode_index == 3;

        container_row_clone.set_visible(is_exec);
        image_row_clone.set_visible(is_run_or_dev);
        shell_row_clone.set_visible(is_docker);
        auto_remove_check_clone.set_visible(is_run_or_dev);
        project_row_clone.set_visible(is_devcontainer);

        // Update Docker status when mode changes
        update_docker_status(&status_label_clone, is_docker);
    });

    // Initial visibility
    container_row.set_visible(false);
    image_row.set_visible(false);
    shell_row.set_visible(false);
    auto_remove_check.set_visible(false);
    project_row.set_visible(false);

    scroll.set_child(Some(&page));

    (
        scroll,
        mode_combo,
        container_entry,
        image_entry,
        shell_entry,
        auto_remove_check,
        project_entry,
        status_label,
        container_row,
        image_row,
        shell_row,
        project_row,
    )
}

#[allow(clippy::type_complexity)]
fn create_ssh_tab() -> (
    ScrolledWindow,
    CheckButton,
    Entry,
    Entry,
    Entry,
    Entry,
    Entry,
    Entry,
    Entry,
    CheckButton,
    CheckButton,
) {
    let scroll = ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let page = GtkBox::new(Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    // SSH enabled checkbox
    let ssh_enabled_check = CheckButton::with_label("Enable SSH (remote connection)");
    page.append(&ssh_enabled_check);

    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);
    grid.set_margin_top(8);

    let mut row = 0;

    // Host
    let host_label = Label::new(Some("Host:"));
    host_label.set_halign(Align::End);
    grid.attach(&host_label, 0, row, 1, 1);
    let host_entry = Entry::new();
    host_entry.set_hexpand(true);
    grid.attach(&host_entry, 1, row, 1, 1);
    row += 1;

    // Port
    let port_label = Label::new(Some("Port:"));
    port_label.set_halign(Align::End);
    grid.attach(&port_label, 0, row, 1, 1);
    let port_entry = Entry::new();
    port_entry.set_placeholder_text(Some("22"));
    port_entry.set_width_request(80);
    grid.attach(&port_entry, 1, row, 1, 1);
    row += 1;

    // Username
    let username_label = Label::new(Some("Username:"));
    username_label.set_halign(Align::End);
    grid.attach(&username_label, 0, row, 1, 1);
    let username_entry = Entry::new();
    grid.attach(&username_entry, 1, row, 1, 1);
    row += 1;

    // Identity file
    let identity_label = Label::new(Some("Identity File:"));
    identity_label.set_halign(Align::End);
    grid.attach(&identity_label, 0, row, 1, 1);
    let identity_entry = Entry::new();
    identity_entry.set_placeholder_text(Some("~/.ssh/id_rsa"));
    grid.attach(&identity_entry, 1, row, 1, 1);
    row += 1;

    // Jump host
    let jump_label = Label::new(Some("Jump Host:"));
    jump_label.set_halign(Align::End);
    grid.attach(&jump_label, 0, row, 1, 1);
    let jump_entry = Entry::new();
    grid.attach(&jump_entry, 1, row, 1, 1);
    row += 1;

    // Local forwards
    let forward_label = Label::new(Some("Local Forwards:"));
    forward_label.set_halign(Align::End);
    grid.attach(&forward_label, 0, row, 1, 1);
    let forward_entry = Entry::new();
    forward_entry.set_placeholder_text(Some("8080:localhost:80, ..."));
    grid.attach(&forward_entry, 1, row, 1, 1);
    row += 1;

    // Remote command
    let remote_cmd_label = Label::new(Some("Remote Cmd:"));
    remote_cmd_label.set_halign(Align::End);
    grid.attach(&remote_cmd_label, 0, row, 1, 1);
    let remote_cmd_entry = Entry::new();
    grid.attach(&remote_cmd_entry, 1, row, 1, 1);

    page.append(&grid);

    // Checkboxes
    let check_box = GtkBox::new(Orientation::Vertical, 4);
    check_box.set_margin_top(12);

    let x11_check = CheckButton::with_label("X11 Forwarding (-X)");
    check_box.append(&x11_check);

    let agent_check = CheckButton::with_label("Agent Forwarding (-A)");
    check_box.append(&agent_check);

    page.append(&check_box);

    // Connect SSH enabled to field sensitivity
    let grid_clone = grid.clone();
    let check_box_clone = check_box.clone();
    ssh_enabled_check.connect_toggled(move |check| {
        let enabled = check.is_active();
        grid_clone.set_sensitive(enabled);
        check_box_clone.set_sensitive(enabled);
    });

    // Initial state: disabled
    grid.set_sensitive(false);
    check_box.set_sensitive(false);

    scroll.set_child(Some(&page));

    (
        scroll,
        ssh_enabled_check,
        host_entry,
        port_entry,
        username_entry,
        identity_entry,
        jump_entry,
        forward_entry,
        remote_cmd_entry,
        x11_check,
        agent_check,
    )
}

fn load_template_into_widgets(widgets: &TemplateWidgets, template: &StickyTabConfig) {
    // General
    widgets.name_entry.set_text(&template.name);
    widgets
        .command_entry
        .set_text(template.command.as_deref().unwrap_or(""));
    widgets.args_entry.set_text(&template.args.join(" "));
    widgets.path_entry.set_text(
        &template
            .working_directory
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    );

    // Color
    if let Some(hex) = &template.color {
        if let Some(rgba) = parse_hex_color(hex) {
            widgets.color_button.set_rgba(&rgba);
        }
    } else {
        widgets
            .color_button
            .set_rgba(&gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    }

    widgets
        .theme_entry
        .set_text(template.theme.as_deref().unwrap_or(""));
    widgets.unique_check.set_active(template.unique);
    widgets.auto_start_check.set_active(template.auto_start);
    widgets.keep_open_check.set_active(template.keep_open);

    // Docker
    let docker_mode = match &template.docker {
        None => 0,
        Some(d) => match d.mode {
            DockerMode::Exec => 1,
            DockerMode::Run => 2,
            DockerMode::DevContainer => 3,
        },
    };
    widgets.docker_mode_combo.set_active(Some(docker_mode));

    // Update visibility based on mode
    let is_docker = docker_mode > 0;
    let is_exec = docker_mode == 1;
    let is_run_or_dev = docker_mode >= 2;
    let is_devcontainer = docker_mode == 3;

    widgets.docker_container_row.set_visible(is_exec);
    widgets.docker_image_row.set_visible(is_run_or_dev);
    widgets.docker_shell_row.set_visible(is_docker);
    widgets.docker_auto_remove_check.set_visible(is_run_or_dev);
    widgets.docker_project_row.set_visible(is_devcontainer);

    if let Some(docker) = &template.docker {
        widgets
            .docker_container_entry
            .set_text(docker.container.as_deref().unwrap_or(""));
        widgets
            .docker_image_entry
            .set_text(docker.image.as_deref().unwrap_or(""));
        widgets
            .docker_shell_entry
            .set_text(docker.shell.as_deref().unwrap_or(""));
        widgets
            .docker_auto_remove_check
            .set_active(docker.auto_remove);
        widgets.docker_project_dir_entry.set_text(
            &docker
                .project_dir
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
    } else {
        widgets.docker_container_entry.set_text("");
        widgets.docker_image_entry.set_text("");
        widgets.docker_shell_entry.set_text("");
        widgets.docker_auto_remove_check.set_active(true);
        widgets.docker_project_dir_entry.set_text("");
    }

    // SSH
    let ssh_enabled = template.ssh.is_some();
    widgets.ssh_enabled_check.set_active(ssh_enabled);

    if let Some(ssh) = &template.ssh {
        widgets.ssh_host_entry.set_text(&ssh.host);
        widgets
            .ssh_port_entry
            .set_text(&ssh.port.map(|p| p.to_string()).unwrap_or_default());
        widgets
            .ssh_username_entry
            .set_text(ssh.username.as_deref().unwrap_or(""));
        widgets.ssh_identity_entry.set_text(
            &ssh.identity_file
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        widgets
            .ssh_jump_host_entry
            .set_text(ssh.jump_host.as_deref().unwrap_or(""));
        widgets.ssh_local_forward_entry.set_text(
            &ssh.local_forwards
                .iter()
                .map(|f| format!("{}:{}:{}", f.local_port, f.remote_host, f.remote_port))
                .collect::<Vec<_>>()
                .join(", "),
        );
        widgets
            .ssh_remote_command_entry
            .set_text(ssh.remote_command.as_deref().unwrap_or(""));
        widgets.ssh_x11_check.set_active(ssh.x11_forward);
        widgets.ssh_agent_check.set_active(ssh.agent_forward);
    } else {
        widgets.ssh_host_entry.set_text("");
        widgets.ssh_port_entry.set_text("");
        widgets.ssh_username_entry.set_text("");
        widgets.ssh_identity_entry.set_text("");
        widgets.ssh_jump_host_entry.set_text("");
        widgets.ssh_local_forward_entry.set_text("");
        widgets.ssh_remote_command_entry.set_text("");
        widgets.ssh_x11_check.set_active(false);
        widgets.ssh_agent_check.set_active(false);
    }
}

fn save_widgets_to_template(widgets: &TemplateWidgets, template: &mut StickyTabConfig) {
    // General
    template.name = widgets.name_entry.text().to_string();
    let cmd = widgets.command_entry.text().to_string();
    template.command = if cmd.is_empty() { None } else { Some(cmd) };
    let args = widgets.args_entry.text().to_string();
    template.args = if args.is_empty() {
        Vec::new()
    } else {
        args.split_whitespace().map(|s| s.to_string()).collect()
    };
    let path = widgets.path_entry.text().to_string();
    template.working_directory = if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    };

    // Color
    let rgba = widgets.color_button.rgba();
    if rgba.alpha() < 0.1 {
        template.color = None;
    } else {
        template.color = Some(format!(
            "#{:02X}{:02X}{:02X}",
            (rgba.red() * 255.0) as u8,
            (rgba.green() * 255.0) as u8,
            (rgba.blue() * 255.0) as u8
        ));
    }

    let theme = widgets.theme_entry.text().to_string();
    template.theme = if theme.is_empty() { None } else { Some(theme) };
    template.unique = widgets.unique_check.is_active();
    template.auto_start = widgets.auto_start_check.is_active();
    template.keep_open = widgets.keep_open_check.is_active();

    // Docker
    let docker_mode = widgets.docker_mode_combo.active().unwrap_or(0);
    if docker_mode == 0 {
        template.docker = None;
    } else {
        let docker = template.docker.get_or_insert_with(DockerTabConfig::default);
        docker.mode = match docker_mode {
            1 => DockerMode::Exec,
            2 => DockerMode::Run,
            3 => DockerMode::DevContainer,
            _ => DockerMode::Exec,
        };
        let container = widgets.docker_container_entry.text().to_string();
        docker.container = if container.is_empty() {
            None
        } else {
            Some(container)
        };
        let image = widgets.docker_image_entry.text().to_string();
        docker.image = if image.is_empty() { None } else { Some(image) };
        let shell = widgets.docker_shell_entry.text().to_string();
        docker.shell = if shell.is_empty() { None } else { Some(shell) };
        docker.auto_remove = widgets.docker_auto_remove_check.is_active();
        let project = widgets.docker_project_dir_entry.text().to_string();
        docker.project_dir = if project.is_empty() {
            None
        } else {
            Some(PathBuf::from(project))
        };
    }

    // SSH
    if !widgets.ssh_enabled_check.is_active() {
        template.ssh = None;
    } else {
        let ssh = template.ssh.get_or_insert_with(SshTabConfig::default);
        ssh.host = widgets.ssh_host_entry.text().to_string();
        ssh.port = widgets.ssh_port_entry.text().to_string().parse().ok();
        let username = widgets.ssh_username_entry.text().to_string();
        ssh.username = if username.is_empty() {
            None
        } else {
            Some(username)
        };
        let identity = widgets.ssh_identity_entry.text().to_string();
        ssh.identity_file = if identity.is_empty() {
            None
        } else {
            Some(PathBuf::from(identity))
        };
        let jump = widgets.ssh_jump_host_entry.text().to_string();
        ssh.jump_host = if jump.is_empty() { None } else { Some(jump) };
        ssh.local_forwards = parse_port_forwards(&widgets.ssh_local_forward_entry.text());
        let remote_cmd = widgets.ssh_remote_command_entry.text().to_string();
        ssh.remote_command = if remote_cmd.is_empty() {
            None
        } else {
            Some(remote_cmd)
        };
        ssh.x11_forward = widgets.ssh_x11_check.is_active();
        ssh.agent_forward = widgets.ssh_agent_check.is_active();
    }
}

fn connect_field_signals(
    widgets: &Rc<TemplateWidgets>,
    templates: Rc<RefCell<Vec<StickyTabConfig>>>,
) {
    // Connect name entry to update combo box text
    let widgets_clone = Rc::clone(widgets);
    let templates_clone = Rc::clone(&templates);
    widgets.name_entry.connect_changed(move |entry| {
        if let Some(index) = widgets_clone.template_combo.active() {
            let name = entry.text().to_string();
            // Update the combo box item text
            widgets_clone.template_combo.remove(index as i32);
            widgets_clone
                .template_combo
                .insert_text(index as i32, &name);
            widgets_clone.template_combo.set_active(Some(index));

            // Update the template
            let mut templates = templates_clone.borrow_mut();
            if let Some(template) = templates.get_mut(index as usize) {
                template.name = name;
            }
        }
    });
}

fn create_preset_template(preset_index: usize) -> Option<StickyTabConfig> {
    match preset_index {
        1 => Some(StickyTabConfig::claude()),
        2 => Some(StickyTabConfig::claude_devcontainer(None)),
        3 => Some(StickyTabConfig {
            name: "Ubuntu".into(),
            color: Some("#E95420".into()),
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some("ubuntu:latest".into()),
                shell: Some("/bin/bash".into()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }),
        4 => Some(StickyTabConfig {
            name: "Alpine".into(),
            color: Some("#0D597F".into()),
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some("alpine:latest".into()),
                shell: Some("/bin/sh".into()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }),
        5 => Some(StickyTabConfig {
            name: "Node.js".into(),
            color: Some("#339933".into()),
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some("node:20".into()),
                shell: Some("/bin/bash".into()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }),
        6 => Some(StickyTabConfig {
            name: "Python".into(),
            color: Some("#3776AB".into()),
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some("python:3.12".into()),
                shell: Some("/bin/bash".into()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }),
        7 => Some(StickyTabConfig {
            name: "SSH Server".into(),
            color: Some("#22c55e".into()),
            keep_open: true,
            ssh: Some(SshTabConfig {
                host: "hostname".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        8 => Some(StickyTabConfig {
            name: "SSH (Agent Fwd)".into(),
            color: Some("#22c55e".into()),
            keep_open: true,
            ssh: Some(SshTabConfig {
                host: "hostname".into(),
                agent_forward: true,
                ..Default::default()
            }),
            ..Default::default()
        }),
        _ => None,
    }
}

fn parse_hex_color(hex: &str) -> Option<gtk4::gdk::RGBA> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;

    Some(gtk4::gdk::RGBA::new(r, g, b, 1.0))
}

fn parse_port_forwards(input: &str) -> Vec<SshPortForward> {
    if input.is_empty() {
        return Vec::new();
    }

    input
        .split(',')
        .filter_map(|part| {
            let parts: Vec<&str> = part.trim().split(':').collect();
            match parts.len() {
                2 => {
                    let local_port = parts[0].parse().ok()?;
                    let remote_port = parts[1].parse().ok()?;
                    Some(SshPortForward {
                        local_port,
                        remote_host: "localhost".to_string(),
                        remote_port,
                    })
                }
                3 => {
                    let local_port = parts[0].parse().ok()?;
                    let remote_host = parts[1].to_string();
                    let remote_port = parts[2].parse().ok()?;
                    Some(SshPortForward {
                        local_port,
                        remote_host,
                        remote_port,
                    })
                }
                _ => None,
            }
        })
        .collect()
}
