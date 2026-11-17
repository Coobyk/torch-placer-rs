fn torch_clicked(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    crate::game::umbx_torch::click(game, mod_id, button)
}
