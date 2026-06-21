#include "ps2recomp/elf_analyzer.h"
#include "ps2recomp/elf_parser.h"
#include "ps2recomp/sce_symbol_scanner.h"
#include "ps2recomp/types.h"
#include <fstream>
#include <iostream>
#include <string>

void printUsage()
{
    std::cout << "PS2 ELF Analyzer\n";
    std::cout << "A tool to analyze PS2 ELF files and generate TOML configuration for PS2Recomp\n\n";
    std::cout << "Usage: ps2_analyzer <input_elf> <output_toml> [sce_symbol_db_dir]\n";
    std::cout << "  input_elf    Path to the PS2 ELF file\n";
    std::cout << "  output_toml  Path to output TOML configuration file\n";
    std::cout << "  sce_symbol_db_dir  Optional override directory containing symbols.json and tree.json\n";
    std::cout << "                     If omitted, the embedded SCE symbol database is used\n\n";
    std::cout << "Usage: ps2_analyzer dump-sce <input_elf> <output_csv> [sce_symbol_db_dir]\n";
    std::cout << "  Scan the ELF with the SCE symbol-signature database and write every match\n";
    std::cout << "  as CSV (Name,Address,Size,Library). Reuses the same scanner + embedded DB\n";
    std::cout << "  as the full analysis path; runs no other analysis.\n";
}

// Emit every SCE-signature match (name -> address) without running the rest of the
// analyzer pipeline. Reuses ElfParser + SceSymbolScanner so the embedded fingerprint
// database is the single source of truth.
static int dumpSceMatches(const std::string &elfPath,
                          const std::string &csvPath,
                          const std::string &sceSymbolDbPath)
{
    ps2recomp::ElfParser parser(elfPath);
    if (!parser.parse())
    {
        std::cerr << "Failed to parse ELF file: " << elfPath << "\n";
        return 1;
    }

    const std::vector<ps2recomp::Section> sections = parser.getSections();

    ps2recomp::SceSymbolScanner scanner;
    if (!scanner.loadDatabase(sceSymbolDbPath))
    {
        const std::string sourceDescription = sceSymbolDbPath.empty()
                                                  ? std::string("embedded database")
                                                  : sceSymbolDbPath;
        std::cerr << "Failed to load SCE symbol database from " << sourceDescription
                  << ": " << scanner.lastError() << "\n";
        return 1;
    }

    const std::vector<ps2recomp::SceSymbolMatch> matches = scanner.scan(sections);

    std::ofstream out(csvPath);
    if (!out)
    {
        std::cerr << "Failed to open output CSV for writing: " << csvPath << "\n";
        return 1;
    }

    out << "Name,Address,Size,Library\n";
    for (const auto &match : matches)
    {
        out << match.name << ",0x" << std::hex << match.address << std::dec
            << "," << match.size << "," << match.library << "\n";
    }
    out.flush();

    const std::string sourceDescription = sceSymbolDbPath.empty()
                                              ? std::string("embedded database")
                                              : sceSymbolDbPath;
    std::cout << "Dumped " << matches.size() << " SCE symbol match(es) from "
              << sourceDescription << " to " << csvPath << "\n";
    return 0;
}

int main(int argc, char *argv[])
{
    if (argc >= 2 && std::string(argv[1]) == "dump-sce")
    {
        if (argc < 4)
        {
            printUsage();
            return 1;
        }
        std::string elfPath = argv[2];
        std::string csvPath = argv[3];
        std::string sceSymbolDbPath = argc >= 5 ? argv[4] : "";
        try
        {
            return dumpSceMatches(elfPath, csvPath, sceSymbolDbPath);
        }
        catch (const std::exception &e)
        {
            std::cerr << "Error: " << e.what() << "\n";
            return 1;
        }
    }

    if (argc < 3)
    {
        printUsage();
        return 1;
    }

    std::string elfPath = argv[1];
    std::string tomlPath = argv[2];
    std::string sceSymbolDbPath = argc >= 4 ? argv[3] : "";

    std::cout << "PS2 ELF Analyzer\n";
    std::cout << "----------------\n";
    std::cout << "Input ELF: " << elfPath << "\n";
    std::cout << "Output TOML: " << tomlPath << "\n\n";
    if (!sceSymbolDbPath.empty())
    {
        std::cout << "SCE symbol DB: " << sceSymbolDbPath << "\n\n";
    }
    else
    {
        std::cout << "SCE symbol DB: embedded\n\n";
    }

    try
    {
        ps2recomp::ElfAnalyzer analyzer(elfPath);
        if (!sceSymbolDbPath.empty())
        {
            analyzer.setSceSymbolDatabasePath(sceSymbolDbPath);
        }

        if (!analyzer.analyze())
        {
            std::cerr << "Failed to analyze ELF file\n";
            return 1;
        }

        if (!analyzer.generateToml(tomlPath))
        {
            std::cerr << "Failed to generate TOML configuration\n";
            return 1;
        }

        std::cout << "\nAnalysis complete\n";
        std::cout << "TOML configuration has been written to: " << tomlPath << "\n";
        std::cout << "\nYou can now use this configuration with PS2Recomp:\n";
        std::cout << "  ps2recomp " << tomlPath << "\n";

        return 0;
    }
    catch (const std::exception &e)
    {
        std::cerr << "Error: " << e.what() << "\n";
        return 1;
    }
}
