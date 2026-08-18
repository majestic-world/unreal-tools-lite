# Unreal Tools Lite

> Kit desktop para preparar texturas e geodata de Lineage 2.

Aplicação Windows construída com **Tauri 2**, **React** e **Rust**. Reúne em uma única interface os fluxos de texturas UTX, redimensionamento e conversão de geodata.

## Recursos

| Área | O que faz |
| --- | --- |
| **Texturas UTX** | Navega por pacotes, pré-visualiza, exporta, substitui e importa texturas. |
| **Utx Extract** | Extrai texturas de vários pacotes UTX em lote, no formato original ou em PNG. |
| **Redimensionar** | Redimensiona texturas DDS e TGA em lote para uma resolução específica. |
| **Converter Geodata** | Converte regiões entre os formatos L2J, CONV_DAT e L2G. |

Os últimos caminhos usados nas ferramentas são preservados localmente.

## Desenvolvimento

Pré-requisitos: Node.js com pnpm, Rust estável e as ferramentas de compilação C++ do Visual Studio no Windows.

```bash
pnpm install
pnpm tauri dev
```

## Build

```bash
pnpm tauri build
```

O executável de produção é gerado em `src-tauri/target/release/Unreal Tools Lite.exe` e o instalador NSIS em `src-tauri/target/release/bundle/nsis/`.

## Verificação

```bash
pnpm build
cd src-tauri && cargo test
```

---

**Unreal Tools Lite · By Mk**
