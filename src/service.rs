use std::{error::Error, ffi::OsString, time::Duration, sync::mpsc};
use structopt::lazy_static;
use windows_service::{
    service::{
        ServiceAccess, ServiceControl, ServiceErrorControl, ServiceInfo, ServiceStartType,
        ServiceType, ServiceStatus, ServiceState, ServiceControlAccept, ServiceExitCode, ServiceAction, ServiceActionType, ServiceFailureActions, ServiceFailureResetPeriod,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess}, define_windows_service,
};
use crate::dispatch;

const SERVICE_NAME: &str = "Geph";
const SERVICE_DISPLAY_NAME: &str = "Geph";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;
lazy_static::lazy_static! {
    static ref SERVICE_ACCESS: ServiceAccess = ServiceAccess::QUERY_CONFIG
    | ServiceAccess::CHANGE_CONFIG
    | ServiceAccess::START
    | ServiceAccess::DELETE;
}

define_windows_service!(ffi_service_main, my_service_main);

fn my_service_main(args: Vec<OsString>) -> anyhow::Result<()> {
    if let Err(e) = run_service(args) {
        eprintln!("Error running service: {}", e);
    }
    Ok(())
}

fn run_service(args: Vec<OsString>) -> windows_service::Result<()> {
    eprintln!("Running service");
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                shutdown_tx.send(()).expect("Unable to shutdown service");
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    match dispatch() {
        Ok(_) => (),
        Err(e) => eprintln!("Error dispatching client: {:?}", e.source()),
    };

    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}

pub fn start() -> windows_service::Result<()> {
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(_) => (),
        Err(e) => println!("error: {:?}", e.source()),
    };

    Ok(())
}

pub fn install() -> windows_service::Result<()> {
    eprintln!("Intitiating service install");
    let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access);
    match &service_manager {
        Ok(_) => (),
        Err(e) => println!("Error instantiating service manager: {:?}", e.source()),
    }

    let service_binary_path = std::env::current_exe()
        .expect("Error retreiving service path")
        .with_file_name("geph4-client.exe");

    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::OnDemand,
        error_control: ServiceErrorControl::Normal,
        executable_path: service_binary_path,
        launch_arguments: vec![
            OsString::from("sync"),
            OsString::from("auth-password"),
            OsString::from("--username"),
            OsString::from("public5"),
            OsString::from("--password"),
            OsString::from("public5")
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    let service = service_manager?.create_service(&service_info, *SERVICE_ACCESS)?;
    let recovery_actions = vec![
        ServiceAction {
            action_type: ServiceActionType::Restart,
            delay: Duration::from_secs(3),
        },
        ServiceAction {
            action_type: ServiceActionType::Restart,
            delay: Duration::from_secs(30),
        },
        ServiceAction {
            action_type: ServiceActionType::Restart,
            delay: Duration::from_secs(300),
        },
    ];

    let failure_actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(900)),
        reboot_msg: None,
        command: None,
        actions: Some(recovery_actions),
    };

    service
        .update_failure_actions(failure_actions)?;
    service
        .set_failure_actions_on_non_crash_failures(true)?;
    service.set_description(
        "Geph connects you with the censorship-free Internet, even when nothing else works.",
    )?;

    Ok(())
}
