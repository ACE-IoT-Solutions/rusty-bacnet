use super::*;

#[pymethods]
impl BACnetServer {
    /// Add an Analog Input object to the server (before starting).
    #[pyo3(signature = (
        instance,
        name,
        units=62,
        present_value=0.0,
        description="",
        cov_increment=0.0
    ))]
    fn add_analog_input(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        present_value: f32,
        description: &str,
        cov_increment: f32,
    ) -> PyResult<()> {
        let mut ai = AnalogInputObject::new(instance, name, units).map_err(to_py_err)?;
        if !present_value.is_finite() {
            return Err(invalid_registration_metadata(
                "present_value must be finite",
            ));
        }
        ai.set_present_value(present_value);
        ai.set_description(description);
        configure_cov_increment(&mut ai, cov_increment)?;
        self.push_pending(Box::new(ai))
    }

    /// Add a Binary Value object to the server (before starting).
    #[pyo3(signature = (instance, name, present_value=false, description=""))]
    fn add_binary_value(
        &self,
        instance: u32,
        name: &str,
        present_value: bool,
        description: &str,
    ) -> PyResult<()> {
        let mut bv = BinaryValueObject::new(instance, name).map_err(to_py_err)?;
        bv.set_relinquish_default(u32::from(present_value))
            .map_err(to_py_err)?;
        bv.set_description(description);
        self.push_pending(Box::new(bv))
    }

    /// Add an Analog Output object to the server (before starting).
    #[pyo3(signature = (
        instance,
        name,
        units=62,
        present_value=0.0,
        description="",
        cov_increment=0.0
    ))]
    fn add_analog_output(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        present_value: f32,
        description: &str,
        cov_increment: f32,
    ) -> PyResult<()> {
        let mut ao = AnalogOutputObject::new(instance, name, units).map_err(to_py_err)?;
        ao.set_relinquish_default(present_value)
            .map_err(to_py_err)?;
        ao.set_description(description);
        configure_cov_increment(&mut ao, cov_increment)?;
        self.push_pending(Box::new(ao))
    }

    /// Add a Binary Input object to the server (before starting).
    #[pyo3(signature = (instance, name, present_value=false, description=""))]
    fn add_binary_input(
        &self,
        instance: u32,
        name: &str,
        present_value: bool,
        description: &str,
    ) -> PyResult<()> {
        let mut bi = BinaryInputObject::new(instance, name).map_err(to_py_err)?;
        bi.set_present_value(u32::from(present_value));
        bi.set_description(description);
        self.push_pending(Box::new(bi))
    }

    /// Add a Binary Output object to the server (before starting).
    #[pyo3(signature = (instance, name, present_value=false, description=""))]
    fn add_binary_output(
        &self,
        instance: u32,
        name: &str,
        present_value: bool,
        description: &str,
    ) -> PyResult<()> {
        let mut bo = BinaryOutputObject::new(instance, name).map_err(to_py_err)?;
        bo.set_relinquish_default(u32::from(present_value))
            .map_err(to_py_err)?;
        bo.set_description(description);
        self.push_pending(Box::new(bo))
    }

    /// Add a Multi-State Input object to the server (before starting).
    #[pyo3(signature = (
        instance,
        name,
        number_of_states,
        state_text=None,
        present_value=1,
        description=""
    ))]
    fn add_multistate_input(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        state_text: Option<Vec<String>>,
        present_value: u32,
        description: &str,
    ) -> PyResult<()> {
        let mut msi =
            MultiStateInputObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        if !(1..=number_of_states).contains(&present_value) {
            return Err(invalid_registration_metadata(format!(
                "present_value must be in 1..={number_of_states}"
            )));
        }
        msi.set_present_value(present_value);
        msi.set_description(description);
        configure_state_text(&mut msi, number_of_states, state_text)?;
        self.push_pending(Box::new(msi))
    }

    /// Add a Multi-State Output object to the server (before starting).
    #[pyo3(signature = (
        instance,
        name,
        number_of_states,
        state_text=None,
        present_value=1,
        description=""
    ))]
    fn add_multistate_output(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        state_text: Option<Vec<String>>,
        present_value: u32,
        description: &str,
    ) -> PyResult<()> {
        let mut mso =
            MultiStateOutputObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        mso.set_relinquish_default(present_value)
            .map_err(to_py_err)?;
        mso.set_description(description);
        configure_state_text(&mut mso, number_of_states, state_text)?;
        self.push_pending(Box::new(mso))
    }

    /// Add a Multi-State Value object to the server (before starting).
    #[pyo3(signature = (
        instance,
        name,
        number_of_states,
        state_text=None,
        present_value=1,
        description=""
    ))]
    fn add_multistate_value(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        state_text: Option<Vec<String>>,
        present_value: u32,
        description: &str,
    ) -> PyResult<()> {
        let mut msv =
            MultiStateValueObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        msv.set_relinquish_default(present_value)
            .map_err(to_py_err)?;
        msv.set_description(description);
        configure_state_text(&mut msv, number_of_states, state_text)?;
        self.push_pending(Box::new(msv))
    }

    /// Add a Calendar object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_calendar(&self, instance: u32, name: &str) -> PyResult<()> {
        let cal = CalendarObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(cal))
    }

    /// Add a Schedule object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_schedule(&self, instance: u32, name: &str) -> PyResult<()> {
        let sched = ScheduleObject::new(instance, name, PropertyValue::Null).map_err(to_py_err)?;
        self.push_pending(Box::new(sched))
    }

    /// Add a Notification Class object to the server (before starting).
    #[pyo3(signature = (instance, name, notification_class=0))]
    fn add_notification_class(
        &self,
        instance: u32,
        name: &str,
        notification_class: u32,
    ) -> PyResult<()> {
        let mut nc = NotificationClass::new(instance, name).map_err(to_py_err)?;
        nc.notification_class = notification_class;
        self.push_pending(Box::new(nc))
    }

    /// Add a Trend Log object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_trend_log(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let tl = TrendLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(tl))
    }

    /// Add a Loop (PID) object to the server (before starting).
    #[pyo3(signature = (instance, name, output_units=62))]
    fn add_loop(&self, instance: u32, name: &str, output_units: u32) -> PyResult<()> {
        let lp = LoopObject::new(instance, name, output_units).map_err(to_py_err)?;
        self.push_pending(Box::new(lp))
    }

    /// Add an Audit Log object to the server (before starting).
    #[pyo3(signature = (instance, name, storage_path, buffer_size=100))]
    fn add_audit_log(
        &self,
        instance: u32,
        name: &str,
        storage_path: &str,
        buffer_size: u32,
    ) -> PyResult<()> {
        let storage = Arc::new(FileAuditLogPersistence::new(storage_path).map_err(to_py_err)?);
        let al = AuditLogObject::new(instance, name, buffer_size, storage).map_err(to_py_err)?;
        self.push_pending(Box::new(al))
    }

    /// Add an Audit Reporter object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_audit_reporter(&self, instance: u32, name: &str) -> PyResult<()> {
        let ar = AuditReporterObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(ar))
    }
}
