// The /torch usages, for the chat box's own colouring and completion.
fn torch_register_client_commands(commands: &mut Vec<mod_api::Command>) {
    crate::game::umbx_torch::register_client_commands(commands)
}
