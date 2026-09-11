use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn usage(program: &str) {
    eprintln!("osu!mania 4K → UMIGURI UGC 转换器\n\n用法:\n  {program} [选项] <输入.osu> [输出.ugc]\n\n选项:\n  --damage-ln-end  在每个 LN 终点额外添加同位置的 DAMAGE 音符\n  -h, --help       显示帮助\n\n未指定输出文件时，会在输入文件旁生成同名 .ugc 文件。\n四轨映射为：0-3、4-7、8-11、12-15。\n");
}

fn main() -> ExitCode {
    let mut args = env::args_os();
    let program = args
        .next()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut damage_ln_end = false;
    let mut positional = Vec::new();
    for arg in args {
        if arg == "-h" || arg == "--help" {
            usage(&program);
            return ExitCode::SUCCESS;
        } else if arg == "--damage-ln-end" {
            damage_ln_end = true;
        } else if arg.to_string_lossy().starts_with('-') {
            eprintln!("错误：未知选项 {}\n", arg.to_string_lossy());
            usage(&program);
            return ExitCode::from(2);
        } else {
            positional.push(arg);
        }
    }
    if positional.is_empty() || positional.len() > 2 {
        usage(&program);
        return ExitCode::from(2);
    }
    let input = PathBuf::from(&positional[0]);
    let output = positional
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| input.with_extension("ugc"));
    if input == output {
        eprintln!("错误：输入和输出路径不能相同");
        return ExitCode::from(2);
    }

    match osu2ugc::convert_file_with_options(
        &input,
        &output,
        osu2ugc::ConvertOptions { damage_ln_end },
    ) {
        Ok(warnings) => {
            for warning in warnings {
                eprintln!("警告：{}", warning.0);
            }
            println!("转换完成：{}", output.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("转换失败：{error}");
            ExitCode::FAILURE
        }
    }
}
