fn torch_command(game: &mut Game<'_>, command: &str, ran: &mut bool) {
    crate::game::umbx_torch::command(game, command, ran)
}
