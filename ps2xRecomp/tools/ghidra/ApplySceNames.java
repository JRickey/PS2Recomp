// Applies SCE-signature-derived names to a Ghidra program from a CSV.
//
// Input CSV (header row + "Name,Address,Size[,...]"), as emitted by
// `ps2_analyzer dump-sce`. For each row:
//   - if a function already starts at the address, rename it to the SCE name;
//   - if the address is inside an existing function (a finer library function the
//     analyzer's signature DB resolved that Ghidra had merged), create a function at
//     the address (Ghidra splits the enclosing function) and name it;
//   - if the address is in executable memory but not yet a function, disassemble and
//     create a function there (recovering an entry Ghidra missed), then name it.
//
// Reuses the analyzer's embedded SCE signature DB as the single source of truth — this
// script only applies the resulting name->address map, it does not re-derive names.
//
// Headless: pass the CSV path as the first script arg. GUI: falls back to askFile.
// @category PS2Recomp

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.symbol.SourceType;

import java.io.BufferedReader;
import java.io.File;
import java.io.FileReader;

public class ApplySceNames extends GhidraScript {

    private boolean isExecutable(Address address) {
        if (address == null) {
            return false;
        }
        MemoryBlock block = currentProgram.getMemory().getBlock(address);
        return block != null && block.isExecute();
    }

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();

        File csvFile;
        if (args != null && args.length >= 1 && args[0] != null && !args[0].isEmpty()) {
            csvFile = new File(args[0]);
        } else {
            csvFile = askFile("Choose SCE names CSV (Name,Address,Size,...)", "Open");
            if (csvFile == null) {
                return;
            }
        }

        FunctionManager fm = currentProgram.getFunctionManager();

        int renamed = 0;
        int splitCreated = 0;
        int newCreated = 0;
        int skipped = 0;
        int rows = 0;

        try (BufferedReader reader = new BufferedReader(new FileReader(csvFile))) {
            String header = reader.readLine(); // skip header
            if (header == null) {
                println("Empty CSV: " + csvFile.getAbsolutePath());
                return;
            }

            String line;
            while ((line = reader.readLine()) != null && !monitor.isCancelled()) {
                line = line.trim();
                if (line.isEmpty()) {
                    continue;
                }

                String[] cols = line.split(",");
                if (cols.length < 2) {
                    continue;
                }

                String name = cols[0].trim();
                String addrStr = cols[1].trim();
                if (name.isEmpty() || addrStr.isEmpty()) {
                    continue;
                }

                long offset;
                try {
                    String hex = addrStr.startsWith("0x") || addrStr.startsWith("0X")
                        ? addrStr.substring(2) : addrStr;
                    offset = Long.parseLong(hex, 16);
                } catch (NumberFormatException e) {
                    continue;
                }
                rows++;

                Address address = currentProgram.getAddressFactory()
                    .getDefaultAddressSpace().getAddress(offset);

                if (!isExecutable(address)) {
                    skipped++;
                    continue;
                }

                Function existing = fm.getFunctionAt(address);
                if (existing != null) {
                    // Exact start match -> rename.
                    if (!existing.getName().equals(name)) {
                        existing.setName(name, SourceType.IMPORTED);
                        renamed++;
                    }
                    continue;
                }

                boolean insideExisting = fm.getFunctionContaining(address) != null;

                // No function starts here. Ensure there is an instruction (disassemble if
                // needed), then create a function. If the address is inside an existing
                // function, createFunction splits the enclosing body at this boundary.
                Instruction insn = currentProgram.getListing().getInstructionAt(address);
                if (insn == null) {
                    disassemble(address);
                }

                Function created = createFunction(address, name);
                if (created == null) {
                    // createFunction can return null if the body could not be determined;
                    // fall back to a primary label so the name is still recorded.
                    createLabel(address, name, true, SourceType.IMPORTED);
                    skipped++;
                    continue;
                }
                if (!created.getName().equals(name)) {
                    created.setName(name, SourceType.IMPORTED);
                }
                if (insideExisting) {
                    splitCreated++;
                } else {
                    newCreated++;
                }
            }
        }

        println(String.format(
            "ApplySceNames: rows=%d renamed=%d split-created=%d new-created=%d skipped=%d",
            rows, renamed, splitCreated, newCreated, skipped));
    }
}
