use crate::editor::ServiceEditor;
use portfu::pfcore::ServiceRegister;
use portfu::prelude::ServiceGroup;

mod editor;

pub struct PortfuAdmin {
    services: ServiceGroup,
}
impl Default for PortfuAdmin {
    fn default() -> Self {
        Self {
            services: ServiceGroup::default()
                //.wrap() AUTH HERE
                .sub_group(ServiceEditor::default()),
        }
    }
}
impl ServiceRegister for PortfuAdmin {
    fn register(self, service_registry: &mut portfu::pfcore::ServiceRegistry) {
        self.services.register(service_registry);
    }
}
