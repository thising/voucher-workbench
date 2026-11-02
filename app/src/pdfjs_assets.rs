//! Fixed, allowlisted PDF.js assets. Generated from web/pdfjs/manifest.json.
pub fn get(path: &str) -> Option<&'static [u8]> {
    match path {
        "cmaps/LICENSE" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/LICENSE"
        ))),
        "LICENSE" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/LICENSE"
        ))),
        "standard_fonts/LICENSE_FOXIT" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/LICENSE_FOXIT"
        ))),
        "wasm/LICENSE_JBIG2" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/LICENSE_JBIG2"
        ))),
        "standard_fonts/LICENSE_LIBERATION" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/LICENSE_LIBERATION"
        ))),
        "wasm/LICENSE_OPENJPEG" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/LICENSE_OPENJPEG"
        ))),
        "wasm/LICENSE_PDFJS_JBIG2" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/LICENSE_PDFJS_JBIG2"
        ))),
        "wasm/LICENSE_PDFJS_OPENJPEG" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/LICENSE_PDFJS_OPENJPEG"
        ))),
        "wasm/LICENSE_PDFJS_QCMS" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/LICENSE_PDFJS_QCMS"
        ))),
        "wasm/LICENSE_PDFJS_QUICKJS" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/LICENSE_PDFJS_QUICKJS"
        ))),
        "wasm/LICENSE_QCMS" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/LICENSE_QCMS"
        ))),
        "wasm/LICENSE_QUICKJS" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/LICENSE_QUICKJS"
        ))),
        "cmaps/78-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/78-EUC-H.bcmap"
        ))),
        "cmaps/78-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/78-EUC-V.bcmap"
        ))),
        "cmaps/78-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/78-H.bcmap"
        ))),
        "cmaps/78-RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/78-RKSJ-H.bcmap"
        ))),
        "cmaps/78-RKSJ-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/78-RKSJ-V.bcmap"
        ))),
        "cmaps/78-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/78-V.bcmap"
        ))),
        "cmaps/78ms-RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/78ms-RKSJ-H.bcmap"
        ))),
        "cmaps/78ms-RKSJ-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/78ms-RKSJ-V.bcmap"
        ))),
        "cmaps/83pv-RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/83pv-RKSJ-H.bcmap"
        ))),
        "cmaps/90ms-RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/90ms-RKSJ-H.bcmap"
        ))),
        "cmaps/90ms-RKSJ-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/90ms-RKSJ-V.bcmap"
        ))),
        "cmaps/90msp-RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/90msp-RKSJ-H.bcmap"
        ))),
        "cmaps/90msp-RKSJ-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/90msp-RKSJ-V.bcmap"
        ))),
        "cmaps/90pv-RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/90pv-RKSJ-H.bcmap"
        ))),
        "cmaps/90pv-RKSJ-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/90pv-RKSJ-V.bcmap"
        ))),
        "cmaps/Add-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Add-H.bcmap"
        ))),
        "cmaps/Add-RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Add-RKSJ-H.bcmap"
        ))),
        "cmaps/Add-RKSJ-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Add-RKSJ-V.bcmap"
        ))),
        "cmaps/Add-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Add-V.bcmap"
        ))),
        "cmaps/Adobe-CNS1-0.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-CNS1-0.bcmap"
        ))),
        "cmaps/Adobe-CNS1-1.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-CNS1-1.bcmap"
        ))),
        "cmaps/Adobe-CNS1-2.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-CNS1-2.bcmap"
        ))),
        "cmaps/Adobe-CNS1-3.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-CNS1-3.bcmap"
        ))),
        "cmaps/Adobe-CNS1-4.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-CNS1-4.bcmap"
        ))),
        "cmaps/Adobe-CNS1-5.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-CNS1-5.bcmap"
        ))),
        "cmaps/Adobe-CNS1-6.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-CNS1-6.bcmap"
        ))),
        "cmaps/Adobe-CNS1-UCS2.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-CNS1-UCS2.bcmap"
        ))),
        "cmaps/Adobe-GB1-0.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-GB1-0.bcmap"
        ))),
        "cmaps/Adobe-GB1-1.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-GB1-1.bcmap"
        ))),
        "cmaps/Adobe-GB1-2.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-GB1-2.bcmap"
        ))),
        "cmaps/Adobe-GB1-3.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-GB1-3.bcmap"
        ))),
        "cmaps/Adobe-GB1-4.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-GB1-4.bcmap"
        ))),
        "cmaps/Adobe-GB1-5.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-GB1-5.bcmap"
        ))),
        "cmaps/Adobe-GB1-UCS2.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-GB1-UCS2.bcmap"
        ))),
        "cmaps/Adobe-Japan1-0.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Japan1-0.bcmap"
        ))),
        "cmaps/Adobe-Japan1-1.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Japan1-1.bcmap"
        ))),
        "cmaps/Adobe-Japan1-2.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Japan1-2.bcmap"
        ))),
        "cmaps/Adobe-Japan1-3.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Japan1-3.bcmap"
        ))),
        "cmaps/Adobe-Japan1-4.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Japan1-4.bcmap"
        ))),
        "cmaps/Adobe-Japan1-5.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Japan1-5.bcmap"
        ))),
        "cmaps/Adobe-Japan1-6.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Japan1-6.bcmap"
        ))),
        "cmaps/Adobe-Japan1-UCS2.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Japan1-UCS2.bcmap"
        ))),
        "cmaps/Adobe-Korea1-0.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Korea1-0.bcmap"
        ))),
        "cmaps/Adobe-Korea1-1.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Korea1-1.bcmap"
        ))),
        "cmaps/Adobe-Korea1-2.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Korea1-2.bcmap"
        ))),
        "cmaps/Adobe-Korea1-UCS2.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Adobe-Korea1-UCS2.bcmap"
        ))),
        "cmaps/B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/B5-H.bcmap"
        ))),
        "cmaps/B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/B5-V.bcmap"
        ))),
        "cmaps/B5pc-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/B5pc-H.bcmap"
        ))),
        "cmaps/B5pc-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/B5pc-V.bcmap"
        ))),
        "cmaps/CNS-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/CNS-EUC-H.bcmap"
        ))),
        "cmaps/CNS-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/CNS-EUC-V.bcmap"
        ))),
        "cmaps/CNS1-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/CNS1-H.bcmap"
        ))),
        "cmaps/CNS1-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/CNS1-V.bcmap"
        ))),
        "cmaps/CNS2-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/CNS2-H.bcmap"
        ))),
        "cmaps/CNS2-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/CNS2-V.bcmap"
        ))),
        "cmaps/ETen-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/ETen-B5-H.bcmap"
        ))),
        "cmaps/ETen-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/ETen-B5-V.bcmap"
        ))),
        "cmaps/ETenms-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/ETenms-B5-H.bcmap"
        ))),
        "cmaps/ETenms-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/ETenms-B5-V.bcmap"
        ))),
        "cmaps/ETHK-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/ETHK-B5-H.bcmap"
        ))),
        "cmaps/ETHK-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/ETHK-B5-V.bcmap"
        ))),
        "cmaps/EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/EUC-H.bcmap"
        ))),
        "cmaps/EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/EUC-V.bcmap"
        ))),
        "cmaps/Ext-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Ext-H.bcmap"
        ))),
        "cmaps/Ext-RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Ext-RKSJ-H.bcmap"
        ))),
        "cmaps/Ext-RKSJ-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Ext-RKSJ-V.bcmap"
        ))),
        "cmaps/Ext-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Ext-V.bcmap"
        ))),
        "cmaps/GB-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GB-EUC-H.bcmap"
        ))),
        "cmaps/GB-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GB-EUC-V.bcmap"
        ))),
        "cmaps/GB-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GB-H.bcmap"
        ))),
        "cmaps/GB-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GB-V.bcmap"
        ))),
        "cmaps/GBK-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBK-EUC-H.bcmap"
        ))),
        "cmaps/GBK-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBK-EUC-V.bcmap"
        ))),
        "cmaps/GBK2K-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBK2K-H.bcmap"
        ))),
        "cmaps/GBK2K-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBK2K-V.bcmap"
        ))),
        "cmaps/GBKp-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBKp-EUC-H.bcmap"
        ))),
        "cmaps/GBKp-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBKp-EUC-V.bcmap"
        ))),
        "cmaps/GBpc-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBpc-EUC-H.bcmap"
        ))),
        "cmaps/GBpc-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBpc-EUC-V.bcmap"
        ))),
        "cmaps/GBT-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBT-EUC-H.bcmap"
        ))),
        "cmaps/GBT-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBT-EUC-V.bcmap"
        ))),
        "cmaps/GBT-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBT-H.bcmap"
        ))),
        "cmaps/GBT-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBT-V.bcmap"
        ))),
        "cmaps/GBTpc-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBTpc-EUC-H.bcmap"
        ))),
        "cmaps/GBTpc-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/GBTpc-EUC-V.bcmap"
        ))),
        "cmaps/H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/H.bcmap"
        ))),
        "cmaps/Hankaku.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Hankaku.bcmap"
        ))),
        "cmaps/Hiragana.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Hiragana.bcmap"
        ))),
        "cmaps/HKdla-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKdla-B5-H.bcmap"
        ))),
        "cmaps/HKdla-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKdla-B5-V.bcmap"
        ))),
        "cmaps/HKdlb-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKdlb-B5-H.bcmap"
        ))),
        "cmaps/HKdlb-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKdlb-B5-V.bcmap"
        ))),
        "cmaps/HKgccs-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKgccs-B5-H.bcmap"
        ))),
        "cmaps/HKgccs-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKgccs-B5-V.bcmap"
        ))),
        "cmaps/HKm314-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKm314-B5-H.bcmap"
        ))),
        "cmaps/HKm314-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKm314-B5-V.bcmap"
        ))),
        "cmaps/HKm471-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKm471-B5-H.bcmap"
        ))),
        "cmaps/HKm471-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKm471-B5-V.bcmap"
        ))),
        "cmaps/HKscs-B5-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKscs-B5-H.bcmap"
        ))),
        "cmaps/HKscs-B5-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/HKscs-B5-V.bcmap"
        ))),
        "cmaps/Katakana.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Katakana.bcmap"
        ))),
        "cmaps/KSC-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSC-EUC-H.bcmap"
        ))),
        "cmaps/KSC-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSC-EUC-V.bcmap"
        ))),
        "cmaps/KSC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSC-H.bcmap"
        ))),
        "cmaps/KSC-Johab-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSC-Johab-H.bcmap"
        ))),
        "cmaps/KSC-Johab-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSC-Johab-V.bcmap"
        ))),
        "cmaps/KSC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSC-V.bcmap"
        ))),
        "cmaps/KSCms-UHC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSCms-UHC-H.bcmap"
        ))),
        "cmaps/KSCms-UHC-HW-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSCms-UHC-HW-H.bcmap"
        ))),
        "cmaps/KSCms-UHC-HW-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSCms-UHC-HW-V.bcmap"
        ))),
        "cmaps/KSCms-UHC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSCms-UHC-V.bcmap"
        ))),
        "cmaps/KSCpc-EUC-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSCpc-EUC-H.bcmap"
        ))),
        "cmaps/KSCpc-EUC-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/KSCpc-EUC-V.bcmap"
        ))),
        "cmaps/NWP-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/NWP-H.bcmap"
        ))),
        "cmaps/NWP-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/NWP-V.bcmap"
        ))),
        "cmaps/RKSJ-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/RKSJ-H.bcmap"
        ))),
        "cmaps/RKSJ-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/RKSJ-V.bcmap"
        ))),
        "cmaps/Roman.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/Roman.bcmap"
        ))),
        "cmaps/UniCNS-UCS2-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniCNS-UCS2-H.bcmap"
        ))),
        "cmaps/UniCNS-UCS2-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniCNS-UCS2-V.bcmap"
        ))),
        "cmaps/UniCNS-UTF16-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniCNS-UTF16-H.bcmap"
        ))),
        "cmaps/UniCNS-UTF16-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniCNS-UTF16-V.bcmap"
        ))),
        "cmaps/UniCNS-UTF32-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniCNS-UTF32-H.bcmap"
        ))),
        "cmaps/UniCNS-UTF32-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniCNS-UTF32-V.bcmap"
        ))),
        "cmaps/UniCNS-UTF8-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniCNS-UTF8-H.bcmap"
        ))),
        "cmaps/UniCNS-UTF8-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniCNS-UTF8-V.bcmap"
        ))),
        "cmaps/UniGB-UCS2-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniGB-UCS2-H.bcmap"
        ))),
        "cmaps/UniGB-UCS2-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniGB-UCS2-V.bcmap"
        ))),
        "cmaps/UniGB-UTF16-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniGB-UTF16-H.bcmap"
        ))),
        "cmaps/UniGB-UTF16-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniGB-UTF16-V.bcmap"
        ))),
        "cmaps/UniGB-UTF32-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniGB-UTF32-H.bcmap"
        ))),
        "cmaps/UniGB-UTF32-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniGB-UTF32-V.bcmap"
        ))),
        "cmaps/UniGB-UTF8-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniGB-UTF8-H.bcmap"
        ))),
        "cmaps/UniGB-UTF8-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniGB-UTF8-V.bcmap"
        ))),
        "cmaps/UniJIS-UCS2-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UCS2-H.bcmap"
        ))),
        "cmaps/UniJIS-UCS2-HW-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UCS2-HW-H.bcmap"
        ))),
        "cmaps/UniJIS-UCS2-HW-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UCS2-HW-V.bcmap"
        ))),
        "cmaps/UniJIS-UCS2-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UCS2-V.bcmap"
        ))),
        "cmaps/UniJIS-UTF16-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UTF16-H.bcmap"
        ))),
        "cmaps/UniJIS-UTF16-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UTF16-V.bcmap"
        ))),
        "cmaps/UniJIS-UTF32-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UTF32-H.bcmap"
        ))),
        "cmaps/UniJIS-UTF32-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UTF32-V.bcmap"
        ))),
        "cmaps/UniJIS-UTF8-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UTF8-H.bcmap"
        ))),
        "cmaps/UniJIS-UTF8-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS-UTF8-V.bcmap"
        ))),
        "cmaps/UniJIS2004-UTF16-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS2004-UTF16-H.bcmap"
        ))),
        "cmaps/UniJIS2004-UTF16-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS2004-UTF16-V.bcmap"
        ))),
        "cmaps/UniJIS2004-UTF32-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS2004-UTF32-H.bcmap"
        ))),
        "cmaps/UniJIS2004-UTF32-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS2004-UTF32-V.bcmap"
        ))),
        "cmaps/UniJIS2004-UTF8-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS2004-UTF8-H.bcmap"
        ))),
        "cmaps/UniJIS2004-UTF8-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJIS2004-UTF8-V.bcmap"
        ))),
        "cmaps/UniJISPro-UCS2-HW-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJISPro-UCS2-HW-V.bcmap"
        ))),
        "cmaps/UniJISPro-UCS2-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJISPro-UCS2-V.bcmap"
        ))),
        "cmaps/UniJISPro-UTF8-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJISPro-UTF8-V.bcmap"
        ))),
        "cmaps/UniJISX0213-UTF32-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJISX0213-UTF32-H.bcmap"
        ))),
        "cmaps/UniJISX0213-UTF32-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJISX0213-UTF32-V.bcmap"
        ))),
        "cmaps/UniJISX02132004-UTF32-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJISX02132004-UTF32-H.bcmap"
        ))),
        "cmaps/UniJISX02132004-UTF32-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniJISX02132004-UTF32-V.bcmap"
        ))),
        "cmaps/UniKS-UCS2-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniKS-UCS2-H.bcmap"
        ))),
        "cmaps/UniKS-UCS2-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniKS-UCS2-V.bcmap"
        ))),
        "cmaps/UniKS-UTF16-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniKS-UTF16-H.bcmap"
        ))),
        "cmaps/UniKS-UTF16-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniKS-UTF16-V.bcmap"
        ))),
        "cmaps/UniKS-UTF32-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniKS-UTF32-H.bcmap"
        ))),
        "cmaps/UniKS-UTF32-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniKS-UTF32-V.bcmap"
        ))),
        "cmaps/UniKS-UTF8-H.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniKS-UTF8-H.bcmap"
        ))),
        "cmaps/UniKS-UTF8-V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/UniKS-UTF8-V.bcmap"
        ))),
        "cmaps/V.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/V.bcmap"
        ))),
        "cmaps/WP-Symbol.bcmap" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/cmaps/WP-Symbol.bcmap"
        ))),
        "wasm/jbig2_nowasm_fallback.js" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/jbig2_nowasm_fallback.js"
        ))),
        "wasm/openjpeg_nowasm_fallback.js" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/openjpeg_nowasm_fallback.js"
        ))),
        "wasm/quickjs-eval.js" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/quickjs-eval.js"
        ))),
        "README.md" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/README.md"
        ))),
        "pdf.mjs" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/pdf.mjs"
        ))),
        "pdf.renderer.mjs" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/pdf.renderer.mjs"
        ))),
        "pdf.worker.mjs" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/pdf.worker.mjs"
        ))),
        "standard_fonts/FoxitDingbats.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitDingbats.pfb"
        ))),
        "standard_fonts/FoxitFixed.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitFixed.pfb"
        ))),
        "standard_fonts/FoxitFixedBold.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitFixedBold.pfb"
        ))),
        "standard_fonts/FoxitFixedBoldItalic.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitFixedBoldItalic.pfb"
        ))),
        "standard_fonts/FoxitFixedItalic.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitFixedItalic.pfb"
        ))),
        "standard_fonts/FoxitSerif.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitSerif.pfb"
        ))),
        "standard_fonts/FoxitSerifBold.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitSerifBold.pfb"
        ))),
        "standard_fonts/FoxitSerifBoldItalic.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitSerifBoldItalic.pfb"
        ))),
        "standard_fonts/FoxitSerifItalic.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitSerifItalic.pfb"
        ))),
        "standard_fonts/FoxitSymbol.pfb" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/FoxitSymbol.pfb"
        ))),
        "standard_fonts/LiberationSans-Bold.ttf" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/LiberationSans-Bold.ttf"
        ))),
        "standard_fonts/LiberationSans-BoldItalic.ttf" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/LiberationSans-BoldItalic.ttf"
        ))),
        "standard_fonts/LiberationSans-Italic.ttf" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/LiberationSans-Italic.ttf"
        ))),
        "standard_fonts/LiberationSans-Regular.ttf" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/standard_fonts/LiberationSans-Regular.ttf"
        ))),
        "wasm/jbig2.wasm" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/jbig2.wasm"
        ))),
        "wasm/openjpeg.wasm" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/openjpeg.wasm"
        ))),
        "wasm/qcms_bg.wasm" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/qcms_bg.wasm"
        ))),
        "wasm/quickjs-eval.wasm" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/wasm/quickjs-eval.wasm"
        ))),
        "manifest.json" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/web/pdfjs/manifest.json"
        ))),
        _ => None,
    }
}
