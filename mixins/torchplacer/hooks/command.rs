fn torch_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    crate::game::umbx_torch::command(game, command, ran)
}
