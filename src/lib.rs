#![feature(array_into_iter_constructors)]


pub use pulldown_cmark::Parser as _Parser;
pub use pulldown_cmark::*;

mod token;
use token::{Lexer, Token};

use Token::*;

use core::ops::Range;
use core::iter::Peekable;
use std::vec;

pub struct Parser<'a, 'b> {
    source: &'a str,
    events: Peekable<pulldown_cmark::OffsetIter<'a, 'b>>,
    lexer: Peekable<Lexer<'b>>,
    buffer: vec::IntoIter<(Event<'a>, Range<usize>)>,
    wikilinks: bool,
}


enum ParseError {
    Empty,
    ReParse(Range<usize>)
}

impl ParseError {
    /// `error.extend_before(start..end)` returns a new error
    /// that spans from start to the end of the error 
    /// (either end, either the original error end)
    fn extend_before(self, r: Range<usize>) -> ParseError {
        match self {
            Self::Empty => Self::ReParse(r),
            Self::ReParse(r2) => Self::ReParse(r.start..r2.end)
        }
    }
}


impl<'a, 'b> Parser<'a, 'b> {
    /// Creates a new event iterator for a markdown string with given options
    pub fn new_ext(source: &'a str, options: Options, wikilinks: bool) -> Self {
        Self {
            source,
            lexer: Lexer::new_at("", 0).peekable(),
            events: pulldown_cmark::Parser::new_ext(source, options)
                .into_offset_iter()
                .peekable(),
            buffer: Vec::new().into_iter(),
            wikilinks,
        }
    }

    /// Consumes the event iterator and produces an iterator that produces
    /// `(Event, Range)` pairs, where the `Range` value maps to the corresponding
    /// range in the markdown source.
    pub fn into_offset_iter(self) -> OffsetIter<'a, 'b> {
        OffsetIter {inner: self}
    }

    /// `self.peek_token()` returns the `Some(Ok(t))` if we are currently reading 
    /// the token `t` in a text event.
    /// if we are currently reading an event `e`, returns `Some(Err(e))`
    fn peek_token(&mut self) -> Option<Result<&(Token, Range<usize>), (Event<'a>, Range<usize>)>> 
      {
        // the buffer created by the wikilink parser (second pass)
        if let Some((x, r)) = self.buffer.next() {
            return Some(Err((x, r)))
        };

        if self.lexer.peek().is_some(){
            return self.lexer.peek().map(Ok)
        }

        match self.events.next()? {
            (Event::Text(_), r) => {
                let start = r.start;
                let mut end = r.end;
                while let Some((Event::Text(_) ,r2)) = self.events.peek(){
                    end = r2.end;
                    self.events.next();
                };
                self.lexer = Lexer::new_at(&self.source[start..end], r.start)
                    .peekable();
                self.peek_token()
            },
            t => Some(Err(t)),
        }
    }

    /// in `[[url|link]]`, returns `url` and don't consume the `|`
    fn parse_wikilink_first_field(&mut self) -> Result<Range<usize>, ParseError> {
        let start : usize = match self.lexer.peek(){
            Some((_, x)) => x.start,
            None => return Err(ParseError::Empty)
        };
        let mut end: usize = start.clone();
        loop {
            match self.lexer.peek() {
                Some((Pipe, _))| Some((RRBra, _)) => break Ok(start..end),
                Some((_, _)) => {
                    end = self.lexer.next().unwrap().1.end;
                }
                None => return Err(ParseError::ReParse(start..end)),
            }
        }
    }

    /// in `link]]`, returns `link` and don't consume the `]]`
    fn parse_wikilink_alias(&mut self) -> Result<Range<usize>, ParseError>{
        let start : usize = match self.lexer.peek(){
            Some((_, x)) => x.start.clone(),
            None => return Err(ParseError::Empty)
        };
        let mut end: usize = start.clone();
        loop {
            match self.lexer.peek() {
                Some((RRBra, _)) => return Ok(start..end),
                Some((_, _)) => {
                    end = self.lexer.next().unwrap().1.end;
                }
                None => return Err(ParseError::ReParse(start..end)),
            }
        }
    }

    /// parse an entire wikilink, ie one of
    /// - `[[a shortcut url]]`
    /// - `[[a url|with some displayed content]]`
    fn parse_wikilink(&mut self) -> Result<Vec<(Event<'a>, Range<usize>)>, ParseError> {
        let tag_pos = self.lexer.next().unwrap().1;
        let url_pos = self.parse_wikilink_first_field()
            .map_err(|x| x.extend_before(tag_pos.clone()))?;

        let opening_tag = Event::Start(Tag::Link{
                link_type: LinkType::Inline,
                dest_url: self.source[url_pos.clone()].into(),
                title: "wiki".into(),
                id: "".into(),
        });

        let closing_tag = Event::End(TagEnd::Link);

        match self.lexer.next() {
            Some((RRBra, x)) => {
                Ok(vec![
                    (opening_tag, tag_pos.start..x.end),
                    (Event::Text(self.source[url_pos.clone()].into()), url_pos),
                    (closing_tag, tag_pos.start..x.end),
                ])
            },
            Some((Pipe, _)) => {
                let alias_pos = self.parse_wikilink_alias()
                    .map_err(|x| x.extend_before(tag_pos.clone()))?;

                let end = self.lexer.next().unwrap().1.end;
                Ok(vec![
                   (opening_tag, tag_pos.start..end),
                    (Event::Text(self.source[alias_pos.clone()].into()), alias_pos),
                   (closing_tag, tag_pos.start..end),
                ])
            }
            _ => unreachable!()
        }
    }

    // parse a text until the first `[[` (start of wikilink) is encountered.
    // don't consume the `[[`
    fn parse_text(&mut self) -> Range<usize> {
        let start = self.lexer.peek().unwrap().1.start.clone();
        let mut end = start.clone();
        loop {
            match self.lexer.peek() {
                Some((LLBra, _)) | None => return start..end,
                Some((_, _)) => {
                    end = self.lexer.next().unwrap().1.end;
                }
            }
        }
    }

    fn next_with_offset(&mut self) -> Option<(Event::<'a>, Range<usize>)> {

        if !self.wikilinks {
            return self.events.next()
        };

        let token = match self.peek_token()? {
            Ok(x) => x,
            Err(e) => return Some(e),
        };

        match token {
            (LLBra, x) => {
                let _start = x.start.clone();
                match self.parse_wikilink() {
                    Ok(b) => {
                        self.buffer = b.into_iter();
                        self.next_with_offset()
                    },
                    Err(e) => {
                        let r = match e {
                            ParseError::ReParse(r) => r,
                            _ => unreachable!(),
                        };
                        Some((Event::Text(self.source[r.clone()].into()), r))
                    }
                }
            },
            (NewLine, _) => self.next_with_offset(),
            _ => {
                let r = self.parse_text();
                Some((Event::Text(self.source[r.clone()].into()), r))
            }
        }
    }
}


impl<'a, 'b> Iterator for Parser<'a, 'b> {
    type Item = Event<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_with_offset().map(|x| x.0)
    }
}

pub struct OffsetIter<'a, 'b> {
    inner: Parser<'a, 'b>,
}

impl<'a, 'b> Iterator for OffsetIter<'a, 'b> {
    type Item = (Event<'a>, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next_with_offset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulldown_cmark::TagEnd;

    use Event::*;
    use LinkType::*;

    #[test]
    fn parse_no_alias() {
        let s = "here is a wikilink: [[link]]";
        let events: Vec<_> =
            Parser::new_ext(s, Options::all(), true)
            .into_offset_iter()
            .collect();

        println!("{events:?}");
        assert_eq!(events, vec![
                   (Start(Tag::Paragraph), 0..28),
                   (Text("here is a wikilink: ".into()), 0..20),
                   (Start(Tag::Link(Inline, "link".into(), "wiki".into())), 20..28),
                   (Text("link".into()), 22..26),
                   (End(TagEnd::Link), 20..28),
                   (End(TagEnd::Paragraph), 0..28),
        ]);
    }


    #[test]
    fn parse_alias(){
        let s = "[[the url| with a strange content |😈| inside]]";

        let original_events: Vec<_> = 
            pulldown_cmark::Parser::new(s)
            .collect();

        println!("{original_events:?}");

        let events: Vec<_> = 
            Parser::new_ext(s, Options::all(), true)
            .collect();

        println!("{events:?}");
        assert_eq!(
            events,
            vec![
                Start(Tag::Paragraph),
                Start(Tag::Link(Inline, "the url".into(), "wiki".into())), 
                Text(" with a strange content |😈| inside".into()), 
                End(TagEnd::Link),
                End(TagEnd::Paragraph),
            ]
        );
    }

    #[test]
    fn empty_text_events(){
        let s = r#"
| unstyled | styled    |
| :-----:  | ------    |
| a  | **a**  |
| b  | **b**  |
| c  | **c**  |
"#;

        let empty_text_events = _Parser::new_ext(s, Options::all())
            .into_offset_iter()
            .filter(|(x, _)| match x {Event::Text(t) if t.is_empty() => true , _ => false});

        assert_eq!(empty_text_events.count(), 3);

        let _events: Vec<_> = 
            Parser::new_ext(s, Options::all(), true)
            .collect();
    }
}
