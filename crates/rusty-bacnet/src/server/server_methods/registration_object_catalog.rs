use super::*;

#[pymethods]
impl BACnetServer {
    // -----------------------------------------------------------------------
    // Pattern A: new(instance, name) — simple two-param constructors
    // -----------------------------------------------------------------------

    /// Add an Analog Value object to the server (before starting).
    #[pyo3(signature = (
        instance,
        name,
        units=62,
        present_value=0.0,
        description="",
        cov_increment=0.0
    ))]
    fn add_analog_value(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        present_value: f32,
        description: &str,
        cov_increment: f32,
    ) -> PyResult<()> {
        let mut obj = AnalogValueObject::new(instance, name, units).map_err(to_py_err)?;
        obj.set_relinquish_default(present_value)
            .map_err(to_py_err)?;
        obj.set_description(description);
        configure_cov_increment(&mut obj, cov_increment)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Command object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_command(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = CommandObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Timer object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_timer(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimerObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Load Control object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_load_control(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LoadControlObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Program object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_program(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = ProgramObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Lighting Output object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_lighting_output(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LightingOutputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Binary Lighting Output object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_lighting_output(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = BinaryLightingOutputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Channel object to the server (before starting).
    #[pyo3(signature = (instance, name, channel_number))]
    fn add_channel(&self, instance: u32, name: &str, channel_number: u32) -> PyResult<()> {
        let obj = ChannelObject::new(instance, name, channel_number).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Life Safety Point object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_life_safety_point(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LifeSafetyPointObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Life Safety Zone object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_life_safety_zone(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LifeSafetyZoneObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Group object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = GroupObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Global Group object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_global_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = GlobalGroupObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Structured View object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_structured_view(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = StructuredViewObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an inert Notification Forwarder property model before starting.
    ///
    /// Registration does not create a separate notification sender; delivery
    /// remains owned by the native server transaction path.
    #[pyo3(signature = (instance, name))]
    fn add_notification_forwarder(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = NotificationForwarderObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Alert Enrollment object to the server (before starting).
    #[pyo3(signature = (instance, name, initial_source))]
    fn add_alert_enrollment(
        &self,
        instance: u32,
        name: &str,
        initial_source: PyObjectIdentifier,
    ) -> PyResult<()> {
        let obj = AlertEnrollmentObject::new(instance, name, initial_source.to_rust())
            .map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Door object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_door(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessDoorObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Credential object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_credential(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessCredentialObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Point object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_point(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessPointObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Rights object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_rights(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessRightsObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access User object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_user(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessUserObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Zone object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_zone(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessZoneObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Credential Data Input object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_credential_data_input(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = CredentialDataInputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Elevator Group object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_elevator_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = ElevatorGroupObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Escalator object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_escalator(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = EscalatorObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Averaging object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_averaging(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AveragingObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    // -----------------------------------------------------------------------
    // Value types — all take new(instance, name)
    // -----------------------------------------------------------------------

    /// Add an Integer Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_integer_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = IntegerValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Positive Integer Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_positive_integer_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = PositiveIntegerValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Large Analog Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_large_analog_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LargeAnalogValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Character String Value object to the server (before starting).
    #[pyo3(signature = (instance, name, present_value="", description=""))]
    fn add_character_string_value(
        &self,
        instance: u32,
        name: &str,
        present_value: &str,
        description: &str,
    ) -> PyResult<()> {
        let mut obj = CharacterStringValueObject::new(instance, name).map_err(to_py_err)?;
        obj.set_relinquish_default(present_value.to_owned())
            .map_err(to_py_err)?;
        obj.write_property(
            bacnet_types::enums::PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString(description.to_owned()),
            None,
        )
        .map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Octet String Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_octet_string_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = OctetStringValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Bit String Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_bit_string_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = BitStringValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Date Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Time Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_time_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimeValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a DateTime Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_time_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateTimeValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Date Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DatePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Time Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_time_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a DateTime Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_time_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateTimePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    // -----------------------------------------------------------------------
    // Pattern B: new(instance, name, extra_param) — three-param constructors
    // -----------------------------------------------------------------------

    /// Add an Accumulator object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_accumulator(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let obj = AccumulatorObject::new(instance, name, units).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Pulse Converter object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_pulse_converter(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let obj = PulseConverterObject::new(instance, name, units).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a File object to the server (before starting).
    #[pyo3(signature = (instance, name, file_type="application/octet-stream"))]
    fn add_file(&self, instance: u32, name: &str, file_type: &str) -> PyResult<()> {
        let obj = FileObject::new(instance, name, file_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Network Port object to the server (before starting).
    #[pyo3(signature = (instance, name, network_type=0))]
    fn add_network_port(&self, instance: u32, name: &str, network_type: u32) -> PyResult<()> {
        let obj = NetworkPortObject::new(instance, name, network_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Event Enrollment object to the server (before starting).
    #[pyo3(signature = (instance, name, event_type=0))]
    fn add_event_enrollment(&self, instance: u32, name: &str, event_type: u32) -> PyResult<()> {
        let obj = EventEnrollmentObject::new(instance, name, event_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an explicitly configured local-target Staging object before starting.
    #[pyo3(signature = (
        instance,
        name,
        present_value,
        min_present_value,
        units,
        priority_for_writing,
        stages,
        target_references,
        stage_names=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_staging(
        &self,
        instance: u32,
        name: &str,
        present_value: f32,
        min_present_value: f32,
        units: u32,
        priority_for_writing: u8,
        stages: Vec<(f32, Vec<bool>, f32)>,
        target_references: Vec<PyObjectIdentifier>,
        stage_names: Option<Vec<String>>,
    ) -> PyResult<()> {
        let config = staging_config(
            present_value,
            min_present_value,
            units,
            priority_for_writing,
            stages,
            target_references,
            stage_names,
        );
        let obj = StagingObject::new(instance, name, config).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Lift object to the server (before starting).
    #[pyo3(signature = (instance, name, num_floors))]
    fn add_lift(&self, instance: u32, name: &str, num_floors: usize) -> PyResult<()> {
        let obj = LiftObject::new(instance, name, num_floors).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Event Log object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_event_log(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let obj = EventLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Trend Log Multiple object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_trend_log_multiple(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let obj = TrendLogMultipleObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }
}
