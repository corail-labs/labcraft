//! One file being edited: its model in canonical form, the names it interns, and the way back to text.

use labcraft_format::{canonicalize, format, read};
use labcraft_graph::{Decl, File, Function, Id, SourceMap};
use labcraft_types::{Interner, Limits, Name, Position, Report, Span};

use crate::EditError;

pub(crate) struct Edit {
    pub path: String,
    /// The text as it was given, before the operation.
    pub original: String,
    pub names: Interner,
    pub file: File,
    /// Where new ids come from: an id only names a place in the map that made it, and the text is read again at the end.
    ids: SourceMap,
    limits: Limits,
}

impl Edit {
    /// Reads the file and puts it in canonical form. A text that does not parse cannot be edited as a graph
    /// (`file.unparsed`), and a file that holds no function body has nothing a command of 6.7 edits.
    pub fn read(path: &str, text: &str, limits: &Limits) -> Result<Edit, EditError> {
        let limits = limits.clone();
        let mut names = Interner::new();
        let reading = read(text.as_bytes(), &mut names, &limits);
        let Some(file) = reading.file else {
            let why = reading.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
            return Err(EditError::new("file.unparsed", format!("`{path}` does not parse, so it cannot be edited as a graph: {why}")).of("file"));
        };
        let mut changes = Report::for_limits(&limits);
        let file = canonicalize(file, &reading.source_map, &mut names, &mut changes);
        if !matches!(file.decl, Decl::Function(_)) {
            return Err(EditError::invalid("file", format!("`{path}` holds no function with a body")));
        }
        Ok(Edit { path: path.to_owned(), original: text.to_owned(), names, file, ids: SourceMap::default(), limits })
    }

    pub fn function(&self) -> Option<&Function> {
        match &self.file.decl {
            Decl::Function(function) => Some(function),
            Decl::Extern(_) | Decl::Type(_) => None,
        }
    }

    pub fn function_mut(&mut self) -> Option<&mut Function> {
        match &mut self.file.decl {
            Decl::Function(function) => Some(function),
            Decl::Extern(_) | Decl::Type(_) => None,
        }
    }

    /// The function, which `read` made sure of.
    pub fn body_function(&self) -> Result<&Function, EditError> {
        self.function().ok_or_else(|| EditError::internal("the file lost its function"))
    }

    pub fn body_function_mut(&mut self) -> Result<&mut Function, EditError> {
        self.function_mut().ok_or_else(|| EditError::internal("the file lost its function"))
    }

    pub fn intern(&mut self, text: &str) -> Name {
        self.names.intern(text)
    }

    pub fn text(&self, name: Name) -> &str {
        self.names.resolve(name)
    }

    /// A new id, for what the operation adds.
    pub fn id(&mut self) -> Id {
        let nowhere = Position { line: 1, column: 1 };
        self.ids.add(Span { start: nowhere, end: nowhere })
    }

    /// The text of the model in canonical form, read again: what the operation would write.
    pub fn render(&self) -> Result<String, EditError> {
        let written = labcraft_format::write(&self.file, &self.names, &self.limits).map_err(|_| {
            EditError::new("file.too_large", format!("`{}` would be over the limit of {} bytes", self.path, self.limits.max_file_size))
        })?;
        let mut names = Interner::new();
        let formatted = format(written.as_bytes(), &mut names, &self.limits);
        match formatted.text {
            Some(text) => Ok(text),
            None => {
                let why = formatted.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
                Err(EditError::internal(format!("the edited text of `{}` does not read again: {why}", self.path)))
            }
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn finish(self) -> Result<String, EditError> {
        self.render()
    }
}
