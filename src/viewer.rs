use std::collections::{HashMap, VecDeque};

use image::DynamicImage;
use image::imageops::FilterType;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use crate::model::{CommitId, Image};
use crate::{feed, popup, theme};

/// How many pictures stay decoded, so flipping back and forth between recent commits is instant.
const KEPT: usize = 9;
const DESCRIPTION_LINES: u16 = 3;

enum Picture {
    Loading,
    Ready(Box<StatefulProtocol>),
    Failed(String),
}

/// The images fetched for viewing, each drawn the best way the terminal knows.
pub struct Gallery {
    picker: Picker,
    pictures: HashMap<String, Picture>,
    /// Oldest first, for dropping pictures past [`KEPT`].
    order: VecDeque<String>,
}

impl Gallery {
    pub fn new(picker: Picker) -> Self {
        Gallery {
            picker,
            pictures: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// The images of `urls` still to fetch: never asked for, dropped, or failed last time.
    pub fn want(&mut self, urls: &[String]) -> Vec<String> {
        let fetch: Vec<String> = (urls.iter())
            .filter(|url| matches!(self.pictures.get(*url), None | Some(Picture::Failed(_))))
            .cloned()
            .collect();
        for url in &fetch {
            self.order.retain(|kept| kept != url);
            self.order.push_back(url.clone());
            self.pictures.insert(url.clone(), Picture::Loading);
        }
        while self.order.len() > KEPT.max(urls.len()) {
            let Some(at) = self.order.iter().position(|kept| !urls.contains(kept)) else {
                break;
            };
            if let Some(dropped) = self.order.remove(at) {
                self.pictures.remove(&dropped);
            }
        }
        fetch
    }

    /// Takes in an image that finished fetching, unless it was dropped while on its way.
    pub fn fetched(&mut self, url: String, image: Result<DynamicImage, String>) {
        if let Some(picture) = self.pictures.get_mut(&url) {
            *picture = match image {
                Ok(image) => Picture::Ready(Box::new(self.picker.new_resize_protocol(image))),
                Err(error) => Picture::Failed(error),
            };
        }
    }

    /// Shows image `at` of commit `id` in a box over most of `screen`.
    pub fn draw(
        &mut self,
        frame: &mut Frame,
        screen: Rect,
        id: &CommitId,
        images: &[Image],
        at: usize,
    ) {
        let Some(image) = images.get(at) else {
            return;
        };
        let title = match images.len() {
            1 => format!(" {} ", id.short_hash()),
            n => format!(" {} · {} of {n} ", id.short_hash(), at + 1),
        };
        let block = Block::bordered()
            .title(Span::styled(
                title,
                Style::new()
                    .fg(theme::FOREGROUND)
                    .add_modifier(Modifier::BOLD),
            ))
            .padding(Padding::horizontal(1));
        let area = popup::place(
            screen,
            screen.width.saturating_sub(4),
            screen.height.saturating_sub(2),
            None,
        );
        let inner = popup::raise(frame, area, block);

        let size = Line::styled(
            format!("{} × {} px", image.width, image.height),
            theme::muted(),
        );
        let mut description = Vec::new();
        if let Some(text) =
            (image.description.as_deref().map(str::trim)).filter(|text| !text.is_empty())
        {
            feed::wrap(
                &[(text.to_owned(), theme::muted())],
                usize::from(inner.width),
                &mut description,
            );
        }
        description.truncate(usize::from(DESCRIPTION_LINES));
        let said = u16::try_from(description.len()).unwrap_or(DESCRIPTION_LINES);
        let [picture, _, caption, meta] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(said),
            Constraint::Length(1),
        ])
        .areas(inner);
        let description: Vec<Line> = description.into_iter().map(Line::from).collect();
        frame.render_widget(Paragraph::new(description), caption);
        frame.render_widget(size, meta);

        let resize = Resize::Fit(Some(FilterType::Triangle));
        match self.pictures.get_mut(&image.url) {
            Some(Picture::Ready(protocol)) => {
                let fits = protocol.size_for(resize.clone(), picture.as_size());
                let [row] = Layout::vertical([Constraint::Length(fits.height)])
                    .flex(ratatui::layout::Flex::Center)
                    .areas(picture);
                let [cell] = Layout::horizontal([Constraint::Length(fits.width)])
                    .flex(ratatui::layout::Flex::Center)
                    .areas(row);
                frame.render_stateful_widget(
                    StatefulImage::default().resize(resize),
                    cell,
                    protocol.as_mut(),
                );
            }
            Some(Picture::Failed(error)) => notice(
                frame,
                picture,
                vec![
                    Line::styled(
                        format!("can't show this image. {error}"),
                        Style::new().fg(theme::FOREGROUND),
                    ),
                    Line::styled("O opens it in the browser", theme::muted()),
                ],
            ),
            Some(Picture::Loading) | None => {
                notice(
                    frame,
                    picture,
                    vec![Line::styled("loading…", theme::muted())],
                );
            }
        }
    }
}

fn notice(frame: &mut Frame, area: Rect, lines: Vec<Line>) {
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let [middle] = Layout::vertical([Constraint::Length(height)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    frame.render_widget(
        Paragraph::new(lines).centered().wrap(Wrap { trim: true }),
        middle,
    );
}

/// Reads fetched bytes as an image no bigger than `bound` pixels a side, which is as much as
/// the screen can show.
pub fn decode(bytes: &[u8], bound: Size) -> Result<DynamicImage, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|_| "it isn't an image this can read.".to_owned())?;
    let (width, height) = (
        u32::from(bound.width.max(1)),
        u32::from(bound.height.max(1)),
    );
    Ok(if image.width() > width || image.height() > height {
        image.resize(width, height, FilterType::CatmullRom)
    } else {
        image
    })
}
