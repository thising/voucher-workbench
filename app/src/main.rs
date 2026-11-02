use clap::Parser;
use std::{path::PathBuf, process::ExitCode};
use voucher_workbench::launcher::{LaunchPaths, RuntimeDiagnostic, StartupOptions};

#[derive(Parser, Debug)]
#[command(version, about = "本地凭证 PDF 工作台", args_override_self = true)]
struct Args {
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(
        long,
        help = "处理记录目录；Windows默认用户LocalAppData/VoucherWorkbench"
    )]
    state_dir: Option<PathBuf>,
    #[arg(long, help = "标题OCR模型目录；Windows默认程序同级models/title-ocr")]
    model: Option<PathBuf>,
    #[arg(long)]
    runtime: Option<PathBuf>,
    #[arg(long)]
    no_browser: bool,
    #[arg(long, help = "检查并加载本地OCR组件，输出JSON后退出，不创建处理记录")]
    check_runtime: bool,
    #[arg(long, hide = true)]
    browser_command: Option<PathBuf>,
    #[arg(long, hide = true, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=10000))]
    startup_delay_ms: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let paths = LaunchPaths::resolve(args.state_dir, args.model);
    if args.check_runtime {
        let report = RuntimeDiagnostic::inspect(paths, args.runtime.as_deref());
        match serde_json::to_string(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("无法输出诊断结果：{error}");
                return ExitCode::FAILURE;
            }
        }
        return if report.model_ready {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let paths = match paths {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("启动失败：{error:#}");
            return ExitCode::FAILURE;
        }
    };
    match voucher_workbench::server::run_with_options(
        args.port,
        paths.state_dir,
        paths.model,
        args.runtime,
        !args.no_browser,
        StartupOptions {
            browser_command: args.browser_command,
            startup_delay_ms: args.startup_delay_ms,
        },
    )
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("启动失败：{error:#}");
            ExitCode::FAILURE
        }
    }
}
