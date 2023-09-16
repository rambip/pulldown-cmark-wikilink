#![feature(array_into_iter_constructors)]


pub use pulldown_cmark::Parser as _Parser;
pub use pulldown_cmark::OffsetIter as _OffsetIter;
pub use pulldown_cmark::*;

// shadows the original types so that the user don't use the wrong one
pub type Parser<'a, 'b> = ParserOffsetIter<'a, 'b>;
pub type OffsetIter<'a, 'b> = ParserOffsetIter<'a, 'b>;

mod token;
use token::{Lexer, Token};

use Token::*;

use core::ops::Range;
use core::iter::Peekable;
use std::vec;


struct TextJoiner<'a, 'b> {
    source: &'a str,
    parser: Peekable<_OffsetIter<'a, 'b>>,
}

impl<'a, 'b> TextJoiner<'a, 'b> {
    fn new_ext(source: &'a str, options: Options) -> Self {
        Self {
            source,
            parser: _Parser::new_ext(source, options)
                .into_offset_iter()
                .peekable(),
        }
    }
}

impl<'a, 'b> Iterator for TextJoiner<'a, 'b> {
    type Item=(Event<'a>, Range<usize>);
    fn next(&mut self) -> Option<Self::Item> {
        match self.parser.peek()? {
            (Event::Text(x), _) if x.is_empty() => {
                self.parser.next();
                self.next()
            },
            (Event::Text(_), range) => {
                let start = range.start;
                let mut end = range.end;
                while let Some((Event::Text(_), _)) = self.parser.peek() {
                    end = self.parser.next().unwrap().1.end;
                }

                Some((Event::Text(self.source[start..end].into()), start..end))

            },
            _ => self.parser.next()
        }
    }
}

pub struct WikiParser<'a, 'b> {
    source: &'a str,
    lexer: Peekable<Lexer<'b>>,
    buffer: vec::IntoIter<(Event<'a>, Range<usize>)>,
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


impl<'a, 'b> WikiParser<'a, 'b> 
    where 'a: 'b
    {
    pub fn new(source: &'a str, range: Range<usize>) -> Self {
        Self {
            source,
            lexer: Lexer::new_at(&source[range.clone()], range.start).peekable(),
            buffer: Vec::new().into_iter()
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
}

impl<'a, 'b> Iterator for WikiParser<'a, 'b> where 'a: 'b {
    type Item = (Event<'a>, Range<usize>);
    fn next(&mut self) -> Option<Self::Item> {
        // returns the last group of events that was created
        if let Some((e, range)) = self.buffer.next() {
            return Some((e, range))
        };

        // suppress useless newlines
        while let Some((Token::NewLine, _)) = self.lexer.peek() {
            self.lexer.next();
        };

        match self.lexer.peek()? {
            (LLBra, x) => {
                let _start = x.start.clone();
                match self.parse_wikilink() {
                    Ok(b) => {
                        self.buffer = b.into_iter();
                        self.buffer.next()
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
            _ => {
                let r = self.parse_text();
                Some((Event::Text(self.source[r.clone()].into()), r))
            }
        }
    }
}

pub struct ParserOffsetIter<'a, 'b> {
    source: &'a str,
    wikilinks: bool,
    events: TextJoiner<'a, 'b>,
    buffer: vec::IntoIter<(Event<'a>, Range<usize>)>,
    inside_metadata: bool,
    inside_codeblock: bool,
}

impl<'a, 'b> ParserOffsetIter<'a, 'b> {
    /// Creates a new event iterator for a markdown string with given options
    pub fn new_ext(source: &'a str, options: Options, wikilinks: bool) -> Self {
        Self {
            source,
            wikilinks,
            events: TextJoiner::new_ext(source, options),
            buffer: Vec::new().into_iter(),
            inside_metadata: false,
            inside_codeblock: false,
        }
    }

    // /// Consumes the event iterator and produces an iterator that produces
    // /// `(Event, Range)` pairs, where the `Range` value maps to the corresponding
    // /// range in the markdown source.
    // pub fn into_offset_iter(self) -> OffsetIter<'a, 'b> {
    //     OffsetIter {
    //         source: self.source,
    //         wikilinks: self.wikilinks,
    //         events: self.events,
    //         buffer: self.buffer,
    //         inside_metadata: self.inside_metadata,
    //         inside_codeblock: self.inside_codeblock
    //     }
    // }
}


impl<'a, 'b> Iterator for ParserOffsetIter<'a, 'b> {
    type Item = (Event<'a>, Range<usize>);
    fn next(&mut self) -> Option<Self::Item> {
        if !self.wikilinks {
            return Some(self.events.next()?)
        }

        if let Some(x) = self.buffer.next() {
            return Some(x)
        }

        match self.events.next()? {
            (Event::End(TagEnd::MetadataBlock(k)), r) if self.inside_metadata => {
                self.inside_metadata = false;
                Some((Event::End(TagEnd::MetadataBlock(k)), r))
            },
            (Event::End(TagEnd::CodeBlock), r) if self.inside_codeblock => {
                self.inside_codeblock = false;
                Some((Event::End(TagEnd::CodeBlock), r))
            },
            (Event::Text(x), r) if self.inside_metadata || self.inside_codeblock => {
                Some((Event::Text(x), r))
            },
            (Event::Start(Tag::MetadataBlock(k)), r) => {
                self.inside_metadata = true;
                Some((Event::Start(Tag::MetadataBlock(k)), r))
            },
            (Event::Start(Tag::CodeBlock(k)), r) => {
                self.inside_codeblock = true;
                Some((Event::Start(Tag::CodeBlock(k)), r))
            },
            (Event::Text(_), range) => {
                self.buffer = WikiParser::new(self.source, range)
                    .collect::<Vec<_>>()
                    .into_iter();

                Some(self.buffer.next().expect("an empty text should not be possible here"))
            },
            (other, r) => return Some((other, r))
        }
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
            ParserOffsetIter::new_ext(s, Options::all(), true)
            .collect();

        println!("{events:?}");
        assert_eq!(events, vec![
                   (Start(Tag::Paragraph), 0..28),
                   (Text("here is a wikilink: ".into()), 0..20),
                   (Start(Tag::Link{link_type: Inline, dest_url: "link".into(), title: "wiki".into(), id: "".into()}), 
                    20..28),
                   (Text("link".into()), 22..26),
                   (End(TagEnd::Link), 20..28),
                   (End(TagEnd::Paragraph), 0..28),
        ]);
    }

    #[test]
    fn parse_in_metadata() {
        let s = "---\n[[wikilink]]\n---";
        let events: Vec<_> = 
            ParserOffsetIter::new_ext(s, Options::all(), true)
            .map(|(x, _)| x)
            .collect();

        assert_eq!(events,
                   vec![
                       Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)),
                       Text("[[wikilink]]\n".into()),
                       End(TagEnd::MetadataBlock(MetadataBlockKind::YamlStyle))]
                   )
    }


    #[test]
    fn parse_alias(){
        let s = "[[the url| with a strange content |😈| inside]]";

        let original_events: Vec<_> = 
            pulldown_cmark::Parser::new(s)
            .collect();

        println!("{original_events:?}");

        let events: Vec<_> = 
            ParserOffsetIter::new_ext(s, Options::all(), true)
            .map(|(x, _)| x)
            .collect();

        println!("{events:?}");
        assert_eq!(
            events,
            vec![
                Start(Tag::Paragraph),
                Start(Tag::Link{link_type: Inline, dest_url: "the url".into(), title: "wiki".into(), id: "".into()}), 
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
            ParserOffsetIter::new_ext(s, Options::all(), true)
            .collect();
    }

    #[test]
    fn link_after_meta(){
        let s = "---\nmetadata: test\n---\n[[link]]";

        let events: Vec<_> = ParserOffsetIter::new_ext(s, Options::all(), true)
            .map(|(x, _)| x)
            .collect();

        use MetadataBlockKind::*;

        assert_eq!(events, vec![
                   Start(Tag::MetadataBlock(YamlStyle)),
                   Text("metadata: test\n".into()),
                   End(TagEnd::MetadataBlock(YamlStyle)),
                   Start(Tag::Paragraph),
                   Start(Tag::Link { link_type: Inline,
                       dest_url: "link".into(),
                       title: "wiki".into(),
                       id: "".into() }),
                   Text("link".into()),
                   End(TagEnd::Link),
                   End(TagEnd::Paragraph)
        ])
    }

    #[test]
    fn link_after_code(){
        let s = "```code\n```\n[[link]]";

        let events: Vec<_> = ParserOffsetIter::new_ext(s, Options::all(), true)
            .map(|(x, _)| x)
            .collect();

        use CodeBlockKind::*;

        assert_eq!(events, vec![
                   Start(Tag::CodeBlock(Fenced("code".into()))),
                   End(TagEnd::CodeBlock),
                   Start(Tag::Paragraph),
                   Start(Tag::Link { link_type: Inline,
                       dest_url: "link".into(),
                       title: "wiki".into(),
                       id: "".into() }),
                   Text("link".into()),
                   End(TagEnd::Link),
                   End(TagEnd::Paragraph)
        ])
    }


    #[test]
    fn link_in_code(){
        let s = "```\n[[]]\n```";

        let events: Vec<_> = ParserOffsetIter::new_ext(s, Options::all(), true)
            .map(|(x, _)| x)
            .collect();

        assert_eq!(events, vec![
                   Start(Tag::CodeBlock(CodeBlockKind::Fenced("".into()))), 
                   Text("[[]]\n".into()), 
                   End(TagEnd::CodeBlock)
        ])
    }

    #[test]
    fn link_in_math(){
        let s = "$$[[]]$$";

        let events: Vec<_> = ParserOffsetIter::new_ext(s, Options::all(), true)
            .map(|(x, _)| x)
            .collect();

        assert_eq!(events, vec![
            Start(Tag::Paragraph), Math(MathMode::Display, "[[]]".into()), End(TagEnd::Paragraph)
        ])
    }

    #[test]
    fn table(){
        // this is mainly a no-regression test.
        // It has to do with empty text events
        let s = "## Style
| unstyled | styled    |
| :-----:  | ------    |
| bold     | **bold**  |
| italics  | *italics* |
| strike   | ~strike~  |
";

        assert_eq!(ParserOffsetIter::new_ext(s, Options::all(), true).count(),
                43);
    }
}
