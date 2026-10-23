use ratatui::layout::Size;
use ratatui_image::FontSize;
use ttyl::viewer::cells;

const FONT: FontSize = FontSize::new(10, 20);

#[test]
fn a_small_image_takes_the_cells_of_its_own_size() {
    assert_eq!(
        cells(1241, 801, FONT, Size::new(200, 60)),
        Size::new(125, 41)
    );
}

#[test]
fn a_big_image_shrinks_to_the_room_keeping_its_shape() {
    assert_eq!(
        cells(4000, 1000, FONT, Size::new(100, 50)),
        Size::new(100, 13)
    );
    assert_eq!(
        cells(1000, 4000, FONT, Size::new(100, 50)),
        Size::new(25, 50)
    );
}

#[test]
fn a_cramped_screen_still_gets_a_cell() {
    assert_eq!(cells(800, 600, FONT, Size::new(0, 0)), Size::new(1, 1));
    assert_eq!(cells(0, 0, FONT, Size::new(10, 10)), Size::new(1, 1));
}
