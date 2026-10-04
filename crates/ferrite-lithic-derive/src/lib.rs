//! `#[derive(PortList)]`: a port list's shape, from a plain struct.
//!
//! # What this generates, and what it deliberately does not
//!
//! A **shape** — names, widths, and a way to build and read a list of signals —
//! and nothing else. It does not decide which ports are inputs and which are
//! outputs. Hardcaml's ppx does not infer direction either
//! ([`ppx/src/circuit.ml`](https://github.com/jane-street/hardcaml)): the user
//! supplies two separate interface types and the circuit builder sits between
//! them. Inferring direction here would mean a convention on field names, and a
//! convention is exactly what is wrong on the designs that matter — `d` is an
//! input on a FIFO and an output on a load unit.
//!
//! So `#[derive(PortList)]` produces the port names, the widths, and a field
//! walk. Direction is the builder's job: `ferrite_lithic::inputs` and
//! `::outputs`.
//!
//! # Attributes
//!
//! Field-level:
//!
//! | Attribute | Meaning |
//! |---|---|
//! | `#[bits(N)]` | Port width. Defaults to `1`. |
//! | `#[length(N)]` | A collection of `N` ports. Required on a `Vec<Signal>` field. |
//! | `#[rtlname("x")]` | Verilog port name. Defaults to the field name. |
//! | `#[exists]` | The port may be absent. The field type must be `Option<Signal>`. |
//! | `#[clock]` | This port is the module clock. At most one per struct. |
//! | `#[wave_format("hex")]` | `binary`, `hex`, or `decimal`. Defaults to `binary`. |
//!
//! Container-level: `#[rtlprefix("p_")]`, `#[rtlsuffix("_q")]`, `#[rtlmangle]`.
//! These rewrite the names as the *Verilog* port list spells them, so they are
//! applied by `PortList::rtl_names()` and not baked into `PORT_NAMES`. Mangling
//! needs `ferrite_lithic-rtl`'s legalisation, which cannot run in a `const`, and
//! a user who writes `#[rtlmangle]` therefore needs that crate in scope.
//!
//! # Example
//!
//! ```
//! use ferrite_lithic::{inputs, Design, Signal};
//! use ferrite_lithic_derive::PortList;
//!
//! #[derive(PortList)]
//! struct Fifo {
//!     #[clock] clock: Signal,
//!     clear: Signal,
//!     #[bits(8)] d: Signal,
//!     #[exists] ready: Option<Signal>,
//! }
//!
//! # fn main() -> Result<(), ferrite_lithic::PortError> {
//! let design = Design::new();
//! let ports = inputs::<Fifo>(&design)?;
//!
//! assert_eq!(Fifo::PORT_NAMES, ["clock", "clear", "d", "ready"]);
//! assert_eq!(Fifo::PORT_WIDTHS, [1, 1, 8, 1]);
//! assert_eq!(Fifo::PORT_CLOCK, Some(0));
//! assert_eq!(ports.clock.width(), 1);
//!
//! // `ready` is declared, so `inputs` declares it.
//! assert!(ports.ready.is_some());
//! # Ok(())
//! # }
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Error as SynError, Fields, GenericArgument, Ident, LitInt, LitStr, Result,
    Type,
};

/// One field, resolved into the port it declares.
struct Port {
    /// The Rust field name.
    ident: Ident,
    /// What kind of list the field holds.
    kind: Kind,
    /// The port's width in bits.
    width: u32,
    /// The name as declared, with `#[rtlname]` applied.
    name: String,
    /// The waveform format.
    format: Ident,
    /// Whether this is the clock.
    clock: bool,
    /// How many ports this field contributes.
    count: usize,
}

/// How a field holds its ports.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// One `Signal`.
    One,
    /// A `Vec<Signal>` of `#[length(N)]` ports.
    Many,
    /// An `Option<Signal>`, one port, possibly absent.
    Maybe,
}

/// The container's name rewriting, which is the Verilog-facing layer.
struct Rewrites {
    prefix: Option<String>,
    suffix: Option<String>,
    mangle: bool,
}

/// Derives the port-list shape described by a struct's fields.
///
/// See the [crate docs](crate) for the attribute list and what is deliberately
/// left to the caller.
#[proc_macro_derive(
    PortList,
    attributes(
        bits,
        length,
        rtlname,
        rtlprefix,
        rtlsuffix,
        rtlmangle,
        exists,
        clock,
        wave_format
    )
)]
pub fn derive_port_list(input: TokenStream) -> TokenStream {
    match syn::parse::<DeriveInput>(input).and_then(|derive| expand(&derive)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand(derive: &DeriveInput) -> Result<TokenStream2> {
    if !derive.generics.params.is_empty() {
        return Err(SynError::new_spanned(
            &derive.generics,
            "PortList cannot be generic: a port list is a fixed interface, and a \
             generic parameter would change which ports a module has without \
             changing the code that declares them",
        ));
    }
    let fields = match &derive.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return Err(SynError::new_spanned(
                    &derive.ident,
                    "PortList needs a struct with named fields: the field names are \
                     the port names, so a tuple struct has nothing to derive them from",
                ));
            }
        },
        _ => {
            return Err(SynError::new_spanned(
                &derive.ident,
                "PortList can only be derived for a struct",
            ));
        }
    };
    if fields.is_empty() {
        return Err(SynError::new_spanned(
            &derive.ident,
            "PortList needs at least one field: a shape with no ports declares no \
             interface, so there is nothing for the emitter or a testbench to use",
        ));
    }

    let rewrites = Rewrites::parse(&derive.attrs)?;
    let ports = fields.iter().map(Port::parse).collect::<Result<Vec<_>>>()?;

    if ports.iter().filter(|port| port.clock).count() > 1 {
        return Err(SynError::new_spanned(
            &derive.ident,
            "more than one field is marked #[clock]. A module has one clock domain, \
             so naming a second would promise an interface the IR cannot deliver",
        ));
    }

    // Duplicate names are rejected here rather than at `set_name`, where the
    // failure would land in the emitter's error path and would name two indices
    // the caller has never heard of.
    let port_names = flat_names(&ports);
    for (first, name) in port_names.iter().enumerate() {
        if let Some(second) = port_names
            .iter()
            .enumerate()
            .skip(first + 1)
            .find(|(_, other)| *other == name)
            .map(|(index, _)| index)
        {
            return Err(SynError::new_spanned(
                &fields[second].ident,
                format!(
                    "port `{name}` is declared by this field and by `{}`. Two ports \
                     cannot share a name in a Verilog port list",
                    field_name(&fields[first])
                ),
            ));
        }
    }

    let name = &derive.ident;

    let widths: Vec<u32> = ports
        .iter()
        .flat_map(|port| core::iter::repeat_n(port.width, port.count))
        .collect();
    let optional: Vec<bool> = ports
        .iter()
        .flat_map(|port| {
            let flag = port.kind == Kind::Maybe;
            core::iter::repeat_n(flag, port.count)
        })
        .collect();
    let formats: Vec<Ident> = ports
        .iter()
        .flat_map(|port| {
            let format = &port.format;
            core::iter::repeat_n(format_ident!("{}", format), port.count)
        })
        .collect();

    // `PORT_CLOCK` is the index of a *port*, not of a field, so a clock that
    // follows a collection has to be offset by that collection's length.
    // `quote` interpolates `None` as nothing at all, so an absent clock has to
    // be spelled out rather than interpolated.
    let clock_index = ports
        .iter()
        .position(|port| port.clock)
        .map(|field| ports[..field].iter().map(|port| port.count).sum::<usize>())
        .map_or_else(|| quote!(None), |index| quote!(Some(#index)));

    let required: usize = ports.iter().map(Port::required).sum();

    // Each field's first port index, which is what `present` is indexed by and
    // what an optional port looks up.
    let mut starts = Vec::with_capacity(ports.len());
    let mut next = 0usize;
    for port in &ports {
        starts.push(next);
        next += port.count;
    }
    let construction = ports
        .iter()
        .zip(&starts)
        .map(|(port, start)| port.construction(*start));
    let set_present = ports
        .iter()
        .zip(&starts)
        .map(|(port, start)| port.set_present(*start));
    let collect_owned = ports.iter().map(Port::collect_owned);
    let collect_refs = ports.iter().map(Port::collect_refs);
    let rtl_names = rtl_names_body(&rewrites);

    Ok(quote! {
        impl #name {
            /// Every declared port's name, in declaration order. A collection
            /// contributes `name_0`, `name_1`, ...
            pub const PORT_NAMES: &'static [&'static str] = &[#(#port_names),*];

            /// Every declared port's width in bits, in declaration order.
            pub const PORT_WIDTHS: &'static [u32] = &[#(#widths),*];

            /// Which declared ports are `#[exists]`.
            pub const PORT_OPTIONAL: &'static [bool] = &[#(#optional),*];

            /// Every declared port's waveform format.
            pub const PORT_FORMATS: &'static [::ferrite_lithic::WaveFormat] =
                &[#(::ferrite_lithic::WaveFormat::#formats),*];

            /// The index of the `#[clock]` port, if there is one.
            pub const PORT_CLOCK: Option<usize> = #clock_index;

            /// How many ports are not optional, which is how many signals a list
            /// of a module where the optional ports are absent has.
            pub const REQUIRED_PORTS: usize = #required;

            /// Builds the shape from one signal per *present* port, given which
            /// ports this list has.
            ///
            /// `present` is one entry per declared port, so it is never a
            /// shorter or longer list than [`PORT_NAMES`](Self::PORT_NAMES).
            /// Marking a required port absent is an error rather than a silent
            /// skip: a caller that thought a port was optional has a bug, and
            /// this is where it can still be named.
            ///
            /// # Errors
            ///
            /// [`NotOptional`](::ferrite_lithic::PortError::NotOptional) if a
            /// required port is marked absent, [`Arity`](::ferrite_lithic::PortError::Arity)
            /// if either list is the wrong length, and
            /// [`Width`](::ferrite_lithic::PortError::Width) if a signal is not
            /// its declared width.
            pub fn from_pattern(
                __signals: ::std::vec::Vec<::ferrite_lithic::Signal>,
                __present: &[bool],
            ) -> ::core::result::Result<Self, ::ferrite_lithic::PortError> {
                ::ferrite_lithic::__private::check_arity(
                    __present.len(),
                    Self::PORT_WIDTHS.len(),
                )?;
                ::ferrite_lithic::__private::check_optional(
                    __present,
                    Self::PORT_OPTIONAL,
                    Self::PORT_NAMES,
                )?;
                let __wanted = __present.iter().filter(|__here| **__here).count();
                ::ferrite_lithic::__private::check_arity(__signals.len(), __wanted)?;
                let mut __iter = __signals.into_iter();
                Ok(Self { #(#construction),* })
            }

            /// Builds the shape from one signal per declared port.
            ///
            /// # Errors
            ///
            /// [`Arity`](::ferrite_lithic::PortError::Arity) if the count is
            /// wrong, [`Width`](::ferrite_lithic::PortError::Width) if a signal
            /// is not its declared width.
            pub fn from_signals(
                __signals: ::std::vec::Vec<::ferrite_lithic::Signal>,
            ) -> ::core::result::Result<Self, ::ferrite_lithic::PortError> {
                Self::from_pattern(__signals, &::std::vec![true; Self::PORT_WIDTHS.len()])
            }

            /// Builds the shape from one signal per non-optional port, which is
            /// the list for a module that does not have the `#[exists]` ports.
            ///
            /// # Errors
            ///
            /// [`Arity`](::ferrite_lithic::PortError::Arity) if the count is
            /// wrong, [`Width`](::ferrite_lithic::PortError::Width) if a signal
            /// is not its declared width.
            pub fn from_present(
                __signals: ::std::vec::Vec<::ferrite_lithic::Signal>,
            ) -> ::core::result::Result<Self, ::ferrite_lithic::PortError> {
                Self::from_pattern(
                    __signals,
                    &Self::PORT_OPTIONAL.iter().map(|o| !o).collect::<::std::vec::Vec<_>>(),
                )
            }

            /// Which of the declared ports this list has.
            #[must_use]
            pub fn present(&self) -> ::std::vec::Vec<bool> {
                let mut __out = ::std::vec![false; Self::PORT_WIDTHS.len()];
                #(#set_present)*
                __out
            }

            /// This shape's signals, in declaration order. An absent optional
            /// port contributes nothing, so the length is however many ports are
            /// present rather than however many are declared.
            #[must_use]
            pub fn to_signals(&self) -> ::std::vec::Vec<::ferrite_lithic::Signal> {
                let mut __out = ::std::vec::Vec::new();
                #(#collect_owned)*
                __out
            }

            /// Every present signal, borrowed, in declaration order.
            pub fn iter(
                &self,
            ) -> impl ::core::iter::Iterator<Item = &::ferrite_lithic::Signal> {
                let mut __out: ::std::vec::Vec<&::ferrite_lithic::Signal> =
                    ::std::vec::Vec::new();
                #(#collect_refs)*
                __out.into_iter()
            }
        }

        impl ::ferrite_lithic::PortList for #name {
            const PORT_NAMES: &'static [&'static str] = Self::PORT_NAMES;
            const PORT_WIDTHS: &'static [u32] = Self::PORT_WIDTHS;
            const PORT_OPTIONAL: &'static [bool] = Self::PORT_OPTIONAL;
            const PORT_FORMATS: &'static [::ferrite_lithic::WaveFormat] = Self::PORT_FORMATS;
            const PORT_CLOCK: Option<usize> = Self::PORT_CLOCK;

            #[must_use]
            fn required_count() -> usize {
                Self::REQUIRED_PORTS
            }

            #[must_use]
            fn rtl_names() -> ::std::vec::Vec<::std::string::String> {
                #rtl_names
            }

            fn from_pattern(
                signals: ::std::vec::Vec<::ferrite_lithic::Signal>,
                present: &[bool],
            ) -> ::core::result::Result<Self, ::ferrite_lithic::PortError> {
                Self::from_pattern(signals, present)
            }

            fn present(&self) -> ::std::vec::Vec<bool> {
                Self::present(self)
            }

            fn from_signals(
                signals: ::std::vec::Vec<::ferrite_lithic::Signal>,
            ) -> ::core::result::Result<Self, ::ferrite_lithic::PortError> {
                Self::from_signals(signals)
            }

            fn from_present(
                signals: ::std::vec::Vec<::ferrite_lithic::Signal>,
            ) -> ::core::result::Result<Self, ::ferrite_lithic::PortError> {
                Self::from_present(signals)
            }

            fn to_signals(&self) -> ::std::vec::Vec<::ferrite_lithic::Signal> {
                Self::to_signals(self)
            }
        }
    })
}

impl Port {
    /// How many of this field's ports are not optional.
    fn required(&self) -> usize {
        match self.kind {
            Kind::Maybe => 0,
            Kind::One | Kind::Many => self.count,
        }
    }

    /// The struct-literal entry for this field.
    ///
    /// `start` is the field's first port index, which is where `present` says
    /// whether an optional port exists.
    fn construction(&self, start: usize) -> TokenStream2 {
        let ident = &self.ident;
        let port = &self.name;
        match self.kind {
            Kind::One => {
                let width = self.width;
                quote! {
                    #ident: ::ferrite_lithic::__private::take_one(&mut __iter, #port, #width)?
                }
            }
            Kind::Many => {
                let count = self.count;
                let width = self.width;
                quote! {
                    #ident: ::ferrite_lithic::__private::take_many(
                        &mut __iter, #port, #count, #width)?
                }
            }
            Kind::Maybe => {
                let width = self.width;
                quote! {
                    #ident: if __present[#start] {
                        ::core::option::Option::Some(
                            ::ferrite_lithic::__private::take_one(
                                &mut __iter, #port, #width)?,
                        )
                    } else {
                        ::core::option::Option::None
                    }
                }
            }
        }
    }

    /// Records this field's ports in the presence vector.
    fn set_present(&self, start: usize) -> TokenStream2 {
        let ident = &self.ident;
        let end = start + self.count;
        match self.kind {
            Kind::One | Kind::Many => quote! {
                __out[#start..#end].fill(true);
            },
            Kind::Maybe => quote! {
                __out[#start] = ::core::option::Option::is_some(&self.#ident);
            },
        }
    }

    fn collect_owned(&self) -> TokenStream2 {
        let ident = &self.ident;
        match self.kind {
            Kind::One => quote! { __out.push(::core::clone::Clone::clone(&self.#ident)); },
            Kind::Many => quote! {
                __out.extend(::core::iter::IntoIterator::into_iter(
                    ::core::clone::Clone::clone(&self.#ident)));
            },
            Kind::Maybe => quote! {
                if let ::core::option::Option::Some(__signal) = &self.#ident {
                    __out.push(::core::clone::Clone::clone(__signal));
                }
            },
        }
    }

    fn collect_refs(&self) -> TokenStream2 {
        let ident = &self.ident;
        match self.kind {
            Kind::One => quote! { __out.push(&self.#ident); },
            Kind::Many => quote! {
                __out.extend(self.#ident.iter());
            },
            Kind::Maybe => quote! {
                if let ::core::option::Option::Some(__signal) = &self.#ident {
                    __out.push(__signal);
                }
            },
        }
    }

    fn parse(field: &syn::Field) -> Result<Self> {
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| SynError::new_spanned(field, "a port field needs a name"))?;
        let attrs = &field.attrs;

        let width = match find_lit_int(attrs, "bits")? {
            Some(literal) => literal.base10_parse::<u32>()?,
            None => 1,
        };
        if width == 0 {
            return Err(SynError::new_spanned(
                &ident,
                "#[bits(0)] declares a zero-width port. A Verilog port list cannot \
                 declare one, and a zero-width value has nothing to carry",
            ));
        }
        let length = match find_lit_int(attrs, "length")? {
            Some(literal) => Some(literal.base10_parse::<usize>()?),
            None => None,
        };
        let exists = attrs.iter().any(|attr| attr.path().is_ident("exists"));
        let clock = attrs.iter().any(|attr| attr.path().is_ident("clock"));
        let format = match find_lit_str(attrs, "wave_format")? {
            Some(text) => match text.value().as_str() {
                "binary" => format_ident!("Binary"),
                "hex" => format_ident!("Hex"),
                "decimal" => format_ident!("Decimal"),
                other => {
                    return Err(SynError::new_spanned(
                        &ident,
                        format!("unknown wave_format `{other}`: expected binary, hex, or decimal"),
                    ));
                }
            },
            None => format_ident!("Binary"),
        };

        let (kind, count) = match classify(&field.ty) {
            Classified::One => {
                if let Some(literal) = &length {
                    return Err(SynError::new_spanned(
                        literal,
                        format!(
                            "#[length({})] is for a Vec<Signal> collection, but this field is \
                             one Signal. Drop the attribute, or make the field a Vec<Signal>",
                            literal
                        ),
                    ));
                }
                (Kind::One, 1)
            }
            Classified::Many => {
                let count = length.ok_or_else(|| {
                    SynError::new_spanned(
                        &ident,
                        "a Vec<Signal> field needs #[length(N)]: a port list has a fixed \
                         number of ports, so a collection cannot declare how many it holds. \
                         Hardcaml's ppx requires the same",
                    )
                })?;
                if count == 0 {
                    return Err(SynError::new_spanned(
                        &ident,
                        "#[length(0)] declares no ports, so the collection is not a port list \
                         at all. Drop the field",
                    ));
                }
                (Kind::Many, count)
            }
            Classified::Maybe => {
                if let Some(literal) = &length {
                    return Err(SynError::new_spanned(
                        literal,
                        "an Option<Signal> field declares one optional port, so it cannot also \
                         be a collection",
                    ));
                }
                (Kind::Maybe, 1)
            }
        };

        if exists && kind != Kind::Maybe {
            return Err(SynError::new_spanned(
                &ident,
                "#[exists] needs an Option<Signal> field: a port that may be absent has to be \
                 absent-able, and a Signal field has nowhere to record the absence",
            ));
        }

        let name = match find_lit_str(attrs, "rtlname")? {
            Some(text) => text.value(),
            None => ident.to_string(),
        };

        Ok(Self {
            ident,
            kind,
            width,
            name,
            format,
            clock,
            count,
        })
    }
}

/// Every port's declared name, with collections expanded to `_0`, `_1`, ...
fn flat_names(ports: &[Port]) -> Vec<String> {
    let mut names = Vec::with_capacity(ports.len());
    for port in ports {
        match port.kind {
            Kind::Many => {
                for index in 0..port.count {
                    names.push(format!("{}_{index}", port.name));
                }
            }
            Kind::One | Kind::Maybe => names.push(port.name.clone()),
        }
    }
    names
}

fn rtl_names_body(rewrites: &Rewrites) -> TokenStream2 {
    let prefix = rewrites.prefix.as_deref().unwrap_or_default();
    let suffix = rewrites.suffix.as_deref().unwrap_or_default();
    // Built here rather than in the `quote!` body: `quote` interpolates `{name}`
    // inside a string literal, so an inline `format!("{prefix}...")` would try to
    // resolve `prefix` as a *generated* variable. The braces around the
    // placeholder are what reach the generated code.
    let mut pattern = String::from(prefix);
    pattern.push_str("{}");
    pattern.push_str(suffix);
    if rewrites.mangle {
        quote! {
            Self::PORT_NAMES
                .iter()
                .map(|__name| {
                    ::ferrite_lithic_rtl::mangle_port_name(
                        &::std::format!(#pattern, __name),
                    )
                })
                .collect()
        }
    } else {
        quote! {
            Self::PORT_NAMES
                .iter()
                .map(|__name| ::std::format!(#pattern, __name))
                .collect()
        }
    }
}

impl Rewrites {
    fn parse(attrs: &[syn::Attribute]) -> Result<Self> {
        let mut rewrites = Self {
            prefix: None,
            suffix: None,
            mangle: false,
        };
        for attr in attrs {
            if attr.path().is_ident("rtlprefix") {
                rewrites.prefix = Some(lit_str(attr)?);
            } else if attr.path().is_ident("rtlmangle") {
                rewrites.mangle = true;
            } else if attr.path().is_ident("rtlsuffix") {
                rewrites.suffix = Some(lit_str(attr)?);
            }
        }
        Ok(rewrites)
    }
}

enum Classified {
    One,
    Many,
    Maybe,
}

/// Whether a field is a `Signal`, a `Vec<Signal>` or an `Option<Signal>`.
///
/// Matching on the last path segment rather than a full path is deliberate: the
/// field may spell `Signal`, `crate::Signal` or an aliased import, and what
/// matters is that the field holds signals, not which crate they came from. A
/// field of any other type is treated as a single port and rejected later, by
/// the fact that the generated code only accepts [`ferrite_lithic::Signal`].
fn classify(ty: &Type) -> Classified {
    let Type::Path(path) = ty else {
        return Classified::One;
    };
    let Some(segment) = path.path.segments.last() else {
        return Classified::One;
    };
    let syn::PathArguments::AngleBracketed(bracketed) = &segment.arguments else {
        return Classified::One;
    };
    let generic: Vec<&Type> = bracketed
        .args
        .iter()
        .filter_map(|argument| match argument {
            GenericArgument::Type(inner) => Some(inner),
            _ => None,
        })
        .collect();
    let container = segment.ident.to_string();
    match (container.as_str(), generic.as_slice()) {
        ("Vec", [inner]) if is_signal(inner) => Classified::Many,
        ("Option", [inner]) if is_signal(inner) => Classified::Maybe,
        _ => Classified::One,
    }
}

fn is_signal(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Signal")
}

fn find_lit_int(attrs: &[syn::Attribute], name: &str) -> Result<Option<LitInt>> {
    let mut found = None;
    for attr in attrs {
        if !attr.path().is_ident(name) {
            continue;
        }
        if found.is_some() {
            return Err(SynError::new_spanned(
                attr,
                format!("duplicate #[{name}]: a port has one width and one length"),
            ));
        }
        found = Some(attr.parse_args::<LitInt>()?);
    }
    Ok(found)
}

fn find_lit_str(attrs: &[syn::Attribute], name: &str) -> Result<Option<LitStr>> {
    let mut found = None;
    for attr in attrs {
        if !attr.path().is_ident(name) {
            continue;
        }
        if found.is_some() {
            return Err(SynError::new_spanned(attr, format!("duplicate #[{name}]")));
        }
        found = Some(attr.parse_args::<LitStr>()?);
    }
    Ok(found)
}

/// A field's name for an error message. Named fields always have one, but the
/// caller has already rejected the nameless case by the time this is used.
fn field_name(field: &syn::Field) -> String {
    field
        .ident
        .as_ref()
        .map_or_else(|| "<unnamed>".to_string(), ToString::to_string)
}

fn lit_str(attr: &syn::Attribute) -> Result<String> {
    Ok(attr.parse_args::<LitStr>()?.value())
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The message a bad struct produces.
    ///
    /// Checked here rather than in a `trybuild` fixture because the message is
    /// the thing: a derive that refuses `Vec<Signal>` without `#[length(N)]` has
    /// to say *why*, and a compile-fail test only proves it refuses.
    fn refuse(source: &str) -> String {
        let derive = syn::parse_str::<DeriveInput>(source).expect("the fixture must parse");
        match expand(&derive) {
            Ok(_) => panic!("this struct should have been refused:\n{source}"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn a_collection_without_a_length_says_so() {
        let message = refuse("struct S { lanes: Vec<Signal> }");
        assert!(message.contains("#[length(N)]"), "{message}");
    }

    #[test]
    fn a_length_on_a_single_signal_says_which_kind_it_is_for() {
        let message = refuse("struct S { #[length(3)] a: Signal }");
        assert!(message.contains("Vec<Signal>"), "{message}");
    }

    #[test]
    fn two_ports_with_one_name_are_refused_by_name() {
        let message =
            refuse("struct S { #[rtlname(\"x\")] a: Signal, #[rtlname(\"x\")] b: Signal }");
        assert!(message.contains("x"), "{message}");
        assert!(message.contains("a"), "{message}");
    }

    #[test]
    fn two_clocks_are_refused() {
        let message = refuse("struct S { #[clock] a: Signal, #[clock] b: Signal }");
        assert!(message.contains("#[clock]"), "{message}");
    }

    #[test]
    fn exists_on_a_plain_signal_is_refused() {
        let message = refuse("struct S { #[exists] a: Signal }");
        assert!(message.contains("Option<Signal>"), "{message}");
    }

    #[test]
    fn a_zero_width_port_is_refused() {
        let message = refuse("struct S { #[bits(0)] a: Signal }");
        assert!(message.contains("#[bits(0)]"), "{message}");
    }

    #[test]
    fn an_unknown_wave_format_lists_the_there_are() {
        let message = refuse("struct S { #[wave_format(\"roman\")] a: Signal }");
        assert!(message.contains("roman"), "{message}");
        assert!(message.contains("binary, hex, or decimal"), "{message}");
    }

    #[test]
    fn an_empty_collection_is_refused() {
        let message = refuse("struct S { #[length(0)] a: Vec<Signal> }");
        assert!(message.contains("#[length(0)]"), "{message}");
    }

    #[test]
    fn a_generic_shape_is_refused() {
        let message = refuse("struct S<T> { a: Signal, b: T }");
        assert!(message.contains("cannot be generic"), "{message}");
    }

    #[test]
    fn a_tuple_shape_is_refused_because_it_has_no_field_names() {
        let message = refuse("struct S(Signal);");
        assert!(message.contains("named fields"), "{message}");
    }

    #[test]
    fn an_empty_shape_is_refused() {
        let message = refuse("struct S {}");
        assert!(message.contains("at least one field"), "{message}");
    }

    #[test]
    fn a_duplicate_attribute_is_refused() {
        let message = refuse("struct S { #[bits(1)] #[bits(2)] a: Signal }");
        assert!(message.contains("duplicate"), "{message}");
    }

    /// The expansion of a small shape, checked for the parts that are easy to
    /// get wrong and hard to read back from a compiler error: an absent clock has
    /// to spell `None`, because `quote` interpolates `None` as nothing.
    #[test]
    fn an_absent_clock_expands_to_none() {
        let derive = syn::parse_str::<DeriveInput>("struct S { a: Signal }").unwrap();
        let expanded = expand(&derive).unwrap().to_string();
        assert!(
            expanded.contains("PORT_CLOCK : Option < usize > = None"),
            "{expanded}"
        );
    }

    /// `quote` resolves `{name}` inside a string literal as a generated
    /// variable, so the prefix/suffix pattern has to be assembled in the macro.
    /// This is the test that would catch a regression there.
    #[test]
    fn a_rewrite_pattern_survives_into_the_generated_format_call() {
        let derive = syn::parse_str::<DeriveInput>(
            "#[rtlmangle]\n#[rtlprefix(\"p_\")]\nstruct S { #[rtlname(\"a b\")] x: Signal }",
        )
        .unwrap();
        let expanded = expand(&derive).unwrap().to_string();

        // The name override is already in PORT_NAMES, so the generated format
        // only has the container's prefix and the placeholder.
        assert!(
            expanded.contains("PORT_NAMES : & 'static [& 'static str] = & [\"a b\"]"),
            "{expanded}"
        );
        assert!(
            expanded.contains("format ! (\"p_{}\" , __name)"),
            "{expanded}"
        );
        assert!(expanded.contains("mangle_port_name"), "{expanded}");
    }

    #[test]
    fn a_shape_with_no_rewrites_generates_a_plain_passthrough() {
        let derive = syn::parse_str::<DeriveInput>("struct S { a: Signal }").unwrap();
        let expanded = expand(&derive).unwrap().to_string();
        assert!(
            expanded.contains("format ! (\"{}\" , __name)"),
            "{expanded}"
        );
        assert!(!expanded.contains("mangle_port_name"), "{expanded}");
    }

    #[test]
    fn the_clock_index_skips_a_collection_that_precedes_it() {
        let derive = syn::parse_str::<DeriveInput>(
            "struct S { #[bits(2)] #[length(3)] lanes: Vec<Signal>, #[clock] clk: Signal }",
        )
        .unwrap();
        let expanded = expand(&derive).unwrap().to_string();
        assert!(
            expanded.contains("PORT_CLOCK : Option < usize > = Some (3usize)"),
            "{expanded}"
        );
    }
}
