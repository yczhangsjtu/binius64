//! Minimal RV32 ELF32 loader (`vm32::elf`) — M8-C T2.
//!
//! Hand-written little-endian ELF32 parser (no new dependencies): reads the
//! ELF header + program headers, collects `PT_LOAD` segments (Jolt
//! `elf.rs`-style filtering: segments must have a non-zero file size) and
//! produces (program image, initial memory, entry point) for the tracer.

#[derive(Debug, Clone)]
pub struct ElfImage {
	/// Entry point (byte address).
	pub entry: u64,
	/// Word-addressed text/image words (`mem[slot]` for fetch), indexed by
	/// `byte_addr >> 2`, sized to cover the highest executable word.
	pub text: Vec<u32>,
	/// Initial RAM contents: `(word_index, value)` for every initialised word
	/// (from PT_LOAD segments, including .data). Unlisted words stay 0.
	pub init_words: Vec<(usize, u32)>,
}

struct Reader<'a> {
	buf: &'a [u8],
	pos: usize,
}

impl<'a> Reader<'a> {
	fn new(buf: &'a [u8]) -> Self {
		Reader { buf, pos: 0 }
	}
	fn u8(&mut self) -> Result<u8, String> {
		let v = *self.buf.get(self.pos).ok_or("elf: truncated")?;
		self.pos += 1;
		Ok(v)
	}
	fn u16(&mut self) -> Result<u16, String> {
		Ok(u16::from_le_bytes([self.u8()?, self.u8()?]))
	}
	fn u32(&mut self) -> Result<u32, String> {
		Ok(u32::from_le_bytes([self.u8()?, self.u8()?, self.u8()?, self.u8()?]))
	}
	fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
		if self.pos + n > self.buf.len() {
			return Err("elf: truncated".into());
		}
		let s = &self.buf[self.pos..self.pos + n];
		self.pos += n;
		Ok(s)
	}
}

const PT_LOAD: u32 = 1;
const SHT_PROGBITS: u32 = 1;
const SHF_EXECINSTR: u32 = 0x4;
const SHF_ALLOC: u32 = 0x2;

/// Parse a little-endian ELF32 relocatable/executable into a loadable image.
pub fn parse_elf32(bytes: &[u8]) -> Result<ElfImage, String> {
	let mut r = Reader::new(bytes);
	// --- ELF header (52 bytes) ---
	if r.bytes(4)? != [0x7f, b'E', b'L', b'F'] {
		return Err("elf: bad magic".into());
	}
	let ei_class = r.u8()?;
	if ei_class != 1 {
		return Err("elf: not ELF32".into());
	}
	let ei_data = r.u8()?;
	if ei_data != 1 {
		return Err("elf: not little-endian".into());
	}
	r.bytes(10)?; // padding
	let _e_type = r.u16()?;
	let _e_machine = r.u16()?;
	let _e_version = r.u32()?;
	let entry = r.u32()? as u64;
	let e_phoff = r.u32()? as usize;
	let e_shoff = r.u32()? as usize;
	let _e_flags = r.u32()?;
	let _e_ehsize = r.u16()?;
	let e_phentsize = r.u16()? as usize;
	let e_phnum = r.u16()? as usize;
	let e_shentsize = r.u16()? as usize;
	let e_shnum = r.u16()? as usize;
	let _e_shstrndx = r.u16()?;

	// --- program headers: PT_LOAD segments (vaddr + file bytes) ---
	let mut init_words: Vec<(usize, u32)> = Vec::new();
	let mut max_exec_word = 0usize;
	for i in 0..e_phnum {
		r.pos = e_phoff + i * e_phentsize;
		let p_type = r.u32()?;
		let p_offset = r.u32()? as usize;
		let p_vaddr = r.u32()? as u64;
		let _p_paddr = r.u32()?;
		let p_filesz = r.u32()? as usize;
		let _p_memsz = r.u32()?;
		let p_flags = r.u32()?;
		if p_type != PT_LOAD || p_filesz == 0 {
			continue;
		}
		let executable = p_flags & 0x1 != 0; // PF_X
		r.pos = p_offset;
		let data = r.bytes(p_filesz)?;
		// fold into word-addressed init memory (4-aligned chunks; tail bytes zero-padded)
		let mut off = 0usize;
		while off < p_filesz {
			let a = (p_vaddr as usize + off) & !3;
			let k = (a >> 2) as usize;
			let mut w = 0u32;
			for b in 0..4 {
				let byte = data.get(off + b).copied().unwrap_or(0);
				w |= (byte as u32) << (8 * b);
			}
			init_words.push((k, w));
			if executable {
				max_exec_word = max_exec_word.max(k + 1);
			}
			off += 4;
		}
	}

	// --- section headers: executable words for the fetch image ---
	// (.text spans; fall back to PT_LOAD X segments if sections are stripped)
	r.pos = e_shoff;
	let sh0 = r.pos;
	if e_shnum > 0 && sh0 + e_shnum * e_shentsize <= bytes.len() {
		for i in 0..e_shnum {
			r.pos = sh0 + i * e_shentsize;
			let sh_name = r.u32()?;
			let sh_type = r.u32()?;
			let sh_flags = r.u32()?;
			let sh_addr = r.u32()? as u64;
			let sh_offset = r.u32()? as usize;
			let sh_size = r.u32()? as usize;
			let _ = sh_name;
			if sh_type != SHT_PROGBITS || (sh_flags & SHF_EXECINSTR) == 0 || (sh_flags & SHF_ALLOC) == 0 {
				continue;
			}
			r.pos = sh_offset;
			let data = r.bytes(sh_size)?;
			let mut off = 0usize;
			while off < sh_size {
				let a = (sh_addr as usize + off) & !3;
				let k = a >> 2;
				let mut w = 0u32;
				for b in 0..4 {
					let byte = data.get(off + b).copied().unwrap_or(0);
					w |= (byte as u32) << (8 * b);
				}
				init_words.push((k, w));
				max_exec_word = max_exec_word.max(k + 1);
				off += 4;
			}
		}
	}

	let text_len = max_exec_word.max(((entry as usize) >> 2) + 1);
	// executable words (from sections, or X-type LOAD segments) override the default.
	let mut text_vals = vec![0x0000_0073u32; text_len];
	for (k, w) in init_words.iter() {
		if *k < text_len {
			text_vals[*k] = *w;
		}
	}
	Ok(ElfImage {
		entry,
		text: text_vals,
		init_words: init_words.into_iter().filter(|(k, _)| *k >= text_len).collect(),
	})
}

/// Convenience: word-index fetch closure for `run_program_big`.
pub fn elf_fetch(img: &ElfImage) -> impl Fn(u64) -> u64 + '_ {
	move |pc: u64| {
		let slot = (pc >> 2) as usize;
		img.text.get(slot).copied().unwrap_or(0x0000_0073) as u64
	}
}

