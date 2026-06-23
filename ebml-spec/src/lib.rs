extern crate proc_macro;

use proc_macro::TokenStream;
use std::collections::HashMap;
use std::fs::read_to_string;
use std::io::Read;
use std::path::Path;
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use quote::{quote, TokenStreamExt};
use syn::{parse_macro_input, LitStr};


#[derive(Debug, Clone)]
enum EbmlType {
    SignedInteger,
    UnsignedInteger,
    Float,
    String,
    UTF8,
    Date,
    Master,
    Binary,
    Void,
    Unsupported,
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    path: String,
    id: String,
    ebml_type: EbmlType,
    default: Option<String>,
    minOccurs: Option<i32>,
    maxOccurs: Option<i32>,
}

#[proc_macro]
pub fn parse_xml(input: TokenStream) -> TokenStream {
    let base_path = input.clone().into_iter().next().unwrap().span().local_file().unwrap();
    let input_string = parse_macro_input!(input as LitStr);
    let filename = input_string.value();

    let path = Path::new(&base_path)
        .parent().unwrap()
        .join(Path::new(&filename));

    println!("{:?}", path);
    let xml = read_to_string(path).unwrap();

    let _element_ids: HashMap<String, String> = HashMap::new();
    let mut elements: Vec<Element> = Vec::new();


    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);

    let _count = 0;
    let mut txt = Vec::new();

    loop {
        match reader.read_event() {
            Err(e) => panic!("Error at position {}: {:?}", reader.error_position(), e),
            Ok(Event::Eof) => break,

            Ok(Event::Start(e)) => {
                if e.name().as_ref() == b"element" {
                    let _bytes = e.bytes();
                    let _as_str =  reader.read_text(e.name());

                    let mut name: String = String::new();
                    let mut path: String = String::new();
                    let mut id: String = String::new();
                    let mut utype: String = String::new();
                    let mut default: Option<String> = None;
                    let mut minOccurs: Option<i32> = None;
                    let mut maxOccurs: Option<i32> = None;

                    for attr_result in e.attributes() {
                        let a = attr_result.unwrap();
                        match a.key.as_ref() {
                            b"name" => {
                                name =
                                    a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap()
                            }
                            b"path" => {
                                path =
                                    a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap()
                            }
                            b"id" => {
                                id =
                                    a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap()
                            }
                            b"type" => {
                                utype =
                                    a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap()
                            }
                            b"default" => {
                                default =
                                    Some(a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap())
                            }
                            b"minOccurs" => {
                                minOccurs =
                                    Some(a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap())
                            }
                            b"maxOccurs" => {
                                maxOccurs =
                                    Some(a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap())
                            }
                            _ => (),
                        }
                    }

                    let ebml_type = match utype.as_str() {
                        "integer" => EbmlType::SignedInteger,
                        "uinteger" => EbmlType::UnsignedInteger,
                        "float" => EbmlType::Float,
                        "string" => EbmlType::String,
                        "utf-8" => EbmlType::UTF8,
                        "date" => EbmlType::Date,
                        "master" => EbmlType::Master,
                        "binary" => EbmlType::Binary,
                        _ => EbmlType::Unsupported,
                    };

                    let element = Element {
                        name,
                        path,
                        id,
                        ebml_type,
                        default,
                        minOccurs,
                        maxOccurs,
                    };

                    elements.push(element);
                }
            }
            Ok(Event::Text(e)) => txt.push(e.decode().unwrap().into_owned()),
            _ => (),
        }
    }

    let struct_instances = elements.into_iter().enumerate().map(|(_index, element)| {
        let id_slice = element.id.strip_prefix("0x").unwrap();
        let id = u64::from_str_radix(id_slice, 16).unwrap();
        let name = &element.name;
        let path = &element.path;

        let default = match element.default {
            Some(m) => quote! { Some(#m.to_string()) },
            None => quote! { None },
        };

        let minOccurs = match element.minOccurs {
            Some(m) => quote! { Some(#m) },
            None => quote! { None },
        };

        let maxOccurs = match element.maxOccurs {
            Some(m) => quote! { Some(#m) },
            None => quote! { None },
        };

        let ebml_type = match element.ebml_type {
            EbmlType::SignedInteger => quote! { EbmlType::SignedInteger },
            EbmlType::UnsignedInteger => quote! { EbmlType::UnsignedInteger },
            EbmlType::Float => quote! { EbmlType::Float },
            EbmlType::String => quote! { EbmlType::String },
            EbmlType::UTF8 => quote! { EbmlType::UTF8 },
            EbmlType::Date => quote! { EbmlType::Date },
            EbmlType::Master => quote! { EbmlType::Master },
            EbmlType::Binary => quote! { EbmlType::Binary },
            EbmlType::Void => quote! { EbmlType::Void },
            EbmlType::Unsupported => quote! { EbmlType::Unsupported },
        };

        quote! {
            Element {
                name: #name.to_string(),
                path: #path.to_string(),
                id: #id,
                ebml_type: #ebml_type,
                default: #default,
                minOccurs: #minOccurs,
                maxOccurs: #maxOccurs,
            }
        }
    });

    let expanded = quote! {
        vec![
            #( #struct_instances ),*
        ]
    };

    // std::fs::write("ebml_matroska_schema.rs", expanded.to_string()).unwrap();

    TokenStream::from(expanded)
}


#[proc_macro]
pub fn create_consts(input: TokenStream) -> TokenStream {
    let base_path = input.clone().into_iter().next().unwrap().span().local_file().unwrap();
    let input_string = parse_macro_input!(input as LitStr);
    let filename = input_string.value();

    let path = Path::new(&base_path)
        .parent().unwrap()
        .join(Path::new(&filename));

    println!("{:?}", path);
    let xml = read_to_string(path).unwrap();

    // println!("The macro received: {}", xml);

    let _element_ids: HashMap<String, String> = HashMap::new();
    let mut elements: Vec<Element> = Vec::new();


    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);

    let _count = 0;
    let mut txt = Vec::new();

    loop {
        match reader.read_event() {
            Err(e) => panic!("Error at position {}: {:?}", reader.error_position(), e),
            Ok(Event::Eof) => break,

            Ok(Event::Start(e)) => {
                if e.name().as_ref() == b"element" {
                    let _bytes = e.bytes();
                    let _as_str =  reader.read_text(e.name());

                    let mut name: String = String::new();
                    let mut path: String = String::new();
                    let mut id: String = String::new();
                    let mut utype: String = String::new();
                    let mut default: Option<String> = None;
                    let mut minOccurs: Option<i32> = None;
                    let mut maxOccurs: Option<i32> = None;

                    for attr_result in e.attributes() {
                        let a = attr_result.unwrap();
                        match a.key.as_ref() {
                            b"name" => {
                                name =
                                    a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap()
                            }
                            b"path" => {
                                path =
                                    a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap()
                            }
                            b"id" => {
                                id =
                                    a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap()
                            }
                            b"type" => {
                                utype =
                                    a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap()
                            }
                            b"default" => {
                                default =
                                    Some(a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap())
                            }
                            b"minOccurs" => {
                                minOccurs =
                                    Some(a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap())
                            }
                            b"maxOccurs" => {
                                maxOccurs =
                                    Some(a.decoded_and_normalized_value(XmlVersion::Explicit1_0, reader.decoder()).unwrap().parse().unwrap())
                            }
                            _ => (),
                        }
                    }

                    let ebml_type = match utype.as_str() {
                        "integer" => EbmlType::SignedInteger,
                        "uinteger" => EbmlType::UnsignedInteger,
                        "float" => EbmlType::Float,
                        "string" => EbmlType::String,
                        "utf-8" => EbmlType::UTF8,
                        "date" => EbmlType::Date,
                        "master" => EbmlType::Master,
                        "binary" => EbmlType::Binary,
                        _ => EbmlType::Unsupported,
                    };

                    let element = Element {
                        name,
                        path,
                        id,
                        ebml_type,
                        default,
                        minOccurs,
                        maxOccurs,
                    };

                    elements.push(element);
                }
            }
            Ok(Event::Text(e)) => txt.push(e.decode().unwrap().into_owned()),
            _ => (),
        }
    }

    let struct_instances = elements.into_iter().enumerate().map(|(_index, element)| {
        let id_slice = element.id.strip_prefix("0x").unwrap();
        let id = u64::from_str_radix(id_slice, 16).unwrap();
        let name = &element.name;

        let upper_name = format!("ID_{}", name.to_ascii_uppercase());
        let upper_name = quote::format_ident!("{}", upper_name.replace('"', ""));

        quote! {
            pub const #upper_name: u64 = #id;
        }
    });

    let base_constants = "
pub const ID_EBML: u64 = 0x1A45DFA3;
pub const ID_EBMLVERSION: u64 = 0x4286;
pub const ID_EBMLREAD_VERSION: u64 = 0x42F7;
pub const ID_EBMLMAX_IDLENGTH: u64 = 0x42F2;
pub const ID_EBMLMAX_SIZE_LENGTH: u64 = 0x42F3;
pub const ID_DOCTYPE: u64 = 0x4282;
pub const ID_DOCTYPE_VERSION: u64 = 0x4287;
pub const ID_DOCTYPE_READ_VERSION: u64 = 0x4285;
pub const ID_DOCTYPE_EXTENSION: u64 = 0x4281;
pub const ID_DOCTYPE_EXTENSION_NAME: u64 = 0x4283;
pub const ID_DOCTYPE_EXTENSION_VERSION: u64 = 0x4284;
pub const ID_CRC32: u64 = 0xBF;
pub const ID_VOID: u64 = 0xEC;";

    let expanded = quote! {
        #( #struct_instances )*
    };

    let joined_constants = format!("{}\n{}", base_constants, expanded);
    let joined_constants = joined_constants.replace("\n", " ");

    println!("{:?}", joined_constants);

    // std::fs::write("ebml_matroska_ids.rs", &joined_constants).unwrap();

    let tk: TokenStream = joined_constants.parse().unwrap();

    tk
}