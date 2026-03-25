//! DWARF debug information generation.
//!
//! Generates `.debug_info`, `.debug_line`, `.debug_abbrev`, and `.debug_str` sections
//! for source-level debugging with `lldb`/`gdb`.

use cranelift_codegen::{Final, MachSrcLoc};
use cranelift_module::FuncId as ClFuncId;
use gimli::write::{
    Address, AttributeValue, DwarfUnit, EndianVec, LineProgram, LineString, Sections,
};
use gimli::{Encoding, Format, LineEncoding, LittleEndian};
use object::write::{Object, Relocation as ObjRelocation, SectionId, SymbolId};
use object::{RelocationEncoding, RelocationFlags, RelocationKind, SectionKind};

/// Source file information for DWARF generation.
pub struct SourceInfo {
    pub filename: String,
    pub directory: String,
    pub source: String,
}

/// Collected debug info for one compiled function.
pub struct FunctionDebugInfo {
    pub name: String,
    pub start_line: u32,
    pub cl_func_id: ClFuncId,
    pub srclocs: Vec<MachSrcLoc<Final>>,
    pub code_size: u64,
}

/// Accumulates debug info during compilation and generates DWARF sections.
pub struct DebugInfoBuilder {
    encoding: Encoding,
    file: SourceInfo,
    functions: Vec<FunctionDebugInfo>,
    /// Map from user symbol index (usize) to object SymbolId.
    symbol_map: Vec<SymbolId>,
}

/// Writer wrapper that tracks relocations for gimli sections.
#[derive(Clone)]
struct DebugSection {
    data: EndianVec<LittleEndian>,
    relocations: Vec<gimli::write::Relocation>,
    obj_section_id: Option<SectionId>,
}

impl DebugSection {
    fn new() -> Self {
        Self {
            data: EndianVec::new(LittleEndian),
            relocations: Vec::new(),
            obj_section_id: None,
        }
    }
}

impl gimli::write::RelocateWriter for DebugSection {
    type Writer = EndianVec<LittleEndian>;

    fn writer(&self) -> &Self::Writer {
        &self.data
    }

    fn writer_mut(&mut self) -> &mut Self::Writer {
        &mut self.data
    }

    fn relocate(&mut self, relocation: gimli::write::Relocation) {
        self.relocations.push(relocation);
    }
}

impl DebugInfoBuilder {
    /// Create a new debug info builder.
    pub fn new(file: SourceInfo, address_size: u8) -> Self {
        Self {
            encoding: Encoding {
                format: Format::Dwarf32,
                version: 4,
                address_size,
            },
            file,
            functions: Vec::new(),
            symbol_map: Vec::new(),
        }
    }

    /// Record debug info for a compiled function.
    pub fn add_function(&mut self, info: FunctionDebugInfo) {
        self.functions.push(info);
    }

    /// Resolve function symbols from ObjectProduct. Must be called before emit_dwarf.
    pub fn resolve_symbols(&mut self, product: &cranelift_object::ObjectProduct) {
        self.symbol_map.clear();
        for func in &self.functions {
            let sym_id = product.function_symbol(func.cl_func_id);
            self.symbol_map.push(sym_id);
        }
    }

    /// Generate DWARF sections and write them into the object file.
    /// Call `resolve_symbols()` first.
    pub fn emit_dwarf(self, obj: &mut Object<'static>) -> Result<(), gimli::write::Error> {
        let func_symbol_indices: Vec<usize> = (0..self.functions.len()).collect();
        let mut dwarf = DwarfUnit::new(self.encoding);

        // Set up the compilation unit root DIE
        let root = dwarf.unit.root();
        {
            let root_die = dwarf.unit.get_mut(root);
            root_die.set(
                gimli::DW_AT_producer,
                AttributeValue::String(b"pyaot AOT Python compiler".to_vec()),
            );
            root_die.set(
                gimli::DW_AT_language,
                AttributeValue::Language(gimli::DW_LANG_Python),
            );
            root_die.set(
                gimli::DW_AT_name,
                AttributeValue::String(self.file.filename.as_bytes().to_vec()),
            );
            root_die.set(
                gimli::DW_AT_comp_dir,
                AttributeValue::String(self.file.directory.as_bytes().to_vec()),
            );
        }

        // Create line program
        let line_encoding = LineEncoding::default();
        let comp_dir = LineString::new(
            self.file.directory.as_bytes().to_vec(),
            self.encoding,
            &mut dwarf.line_strings,
        );
        let comp_file = LineString::new(
            self.file.filename.as_bytes().to_vec(),
            self.encoding,
            &mut dwarf.line_strings,
        );

        let mut line_program = LineProgram::new(
            self.encoding,
            line_encoding,
            comp_dir,
            None,
            comp_file,
            None,
        );

        let dir_id = line_program.default_directory();
        let file_string = LineString::new(
            self.file.filename.as_bytes().to_vec(),
            self.encoding,
            &mut dwarf.line_strings,
        );
        let file_id = line_program.add_file(file_string, dir_id, None);

        // Add subprogram DIEs and line info for each function
        for (func_idx, func) in self.functions.iter().enumerate() {
            if func.srclocs.is_empty() {
                continue;
            }

            let sym_idx = func_symbol_indices[func_idx];

            let func_address = Address::Symbol {
                symbol: sym_idx,
                addend: 0,
            };

            // Add DW_TAG_subprogram DIE
            let subprogram_id = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
            let subprogram = dwarf.unit.get_mut(subprogram_id);
            subprogram.set(
                gimli::DW_AT_name,
                AttributeValue::String(func.name.as_bytes().to_vec()),
            );
            subprogram.set(gimli::DW_AT_external, AttributeValue::Flag(true));
            subprogram.set(
                gimli::DW_AT_decl_file,
                AttributeValue::FileIndex(Some(file_id)),
            );
            subprogram.set(
                gimli::DW_AT_decl_line,
                AttributeValue::Udata(func.start_line as u64),
            );
            subprogram.set(gimli::DW_AT_low_pc, AttributeValue::Address(func_address));
            subprogram.set(gimli::DW_AT_high_pc, AttributeValue::Udata(func.code_size));

            // Add line program entries for this function
            line_program.begin_sequence(Some(func_address));

            let mut prev_line = 0u64;
            for srcloc in &func.srclocs {
                if srcloc.loc.is_default() {
                    continue;
                }
                let line = srcloc.loc.bits() as u64;
                if line == 0 || line == prev_line {
                    continue;
                }

                let row = line_program.row();
                row.file = file_id;
                row.line = line;
                row.column = 0;
                row.address_offset = srcloc.start as u64;
                row.is_statement = true;
                if prev_line == 0 {
                    row.prologue_end = true;
                }
                line_program.generate_row();
                prev_line = line;
            }

            line_program.end_sequence(func.code_size);
        }

        dwarf.unit.line_program = line_program;

        // Write DWARF sections
        let mut sections = Sections::new(DebugSection::new());
        dwarf.write(&mut sections)?;

        // Add sections to object file and process relocations
        let symbol_map = &self.symbol_map;

        // First pass: add section data
        sections.for_each_mut(|id, section| -> Result<(), gimli::write::Error> {
            use gimli::write::{RelocateWriter, Writer};
            if section.writer().len() == 0 {
                return Ok(());
            }

            let section_kind = match id {
                gimli::SectionId::DebugStr | gimli::SectionId::DebugLineStr => {
                    SectionKind::DebugString
                }
                _ => SectionKind::Debug,
            };

            let name = id.name().as_bytes().to_vec();
            let segment = obj
                .segment_name(object::write::StandardSegment::Debug)
                .to_vec();

            let obj_section_id = obj.add_section(segment, name, section_kind);
            let data = section.writer().slice().to_vec();
            obj.set_section_data(obj_section_id, data, 1);
            section.obj_section_id = Some(obj_section_id);
            Ok(())
        })?;

        // Second pass: add relocations
        // We need to collect relocations first since for_each borrows sections
        let mut all_relocs: Vec<(SectionId, Vec<ObjRelocation>)> = Vec::new();

        sections.for_each(|id, section| -> Result<(), gimli::write::Error> {
            let Some(obj_section_id) = section.obj_section_id else {
                return Ok(());
            };

            let mut relocs = Vec::new();
            for reloc in &section.relocations {
                let symbol = match reloc.target {
                    gimli::write::RelocationTarget::Section(section_id) => {
                        // This is a reference to another DWARF section
                        // We need to find the object section and get its symbol
                        // For now, skip section-to-section relocations since gimli
                        // handles most references inline for DWARF4
                        let _ = (id, section_id);
                        continue;
                    }
                    gimli::write::RelocationTarget::Symbol(sym_idx) => {
                        if sym_idx < symbol_map.len() {
                            symbol_map[sym_idx]
                        } else {
                            continue;
                        }
                    }
                };

                relocs.push(ObjRelocation {
                    offset: reloc.offset as u64,
                    symbol,
                    addend: reloc.addend,
                    flags: RelocationFlags::Generic {
                        kind: RelocationKind::Absolute,
                        encoding: RelocationEncoding::Generic,
                        size: reloc.size * 8,
                    },
                });
            }
            if !relocs.is_empty() {
                all_relocs.push((obj_section_id, relocs));
            }
            Ok(())
        })?;

        for (section_id, relocs) in all_relocs {
            for reloc in relocs {
                obj.add_relocation(section_id, reloc)
                    .map_err(|_| gimli::write::Error::InvalidAttributeValue)?;
            }
        }

        Ok(())
    }
}

/// Extract function code size from Cranelift's compiled code context.
pub fn get_compiled_function_size(ctx: &cranelift_codegen::Context) -> Option<u64> {
    ctx.compiled_code().map(|c| c.code_info().total_size as u64)
}

/// Extract sorted source location mappings from compiled code.
pub fn get_compiled_srclocs(ctx: &cranelift_codegen::Context) -> Vec<MachSrcLoc<Final>> {
    ctx.compiled_code()
        .map(|c| c.buffer.get_srclocs_sorted().to_vec())
        .unwrap_or_default()
}
