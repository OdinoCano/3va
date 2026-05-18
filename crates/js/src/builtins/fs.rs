use rquickjs::{Ctx, Result, Function, Object};
use std::rc::Rc;
use std::cell::RefCell;
use vvva_permissions::{PermissionState, Capability};

pub fn inject_fs(ctx: &Ctx, permissions: Rc<RefCell<PermissionState>>) -> Result<()> {
    ctx.eval::<(), _>(r#"
        const fs = {
            readFile: function(path) {
                throw new Error('fs.readFile requires --allow-read');
            },
            writeFile: function(path, content) {
                throw new Error('fs.writeFile requires --allow-write');
            },
            exists: function(path) {
                return false;
            },
            readdir: function(path) {
                throw new Error('fs.readdir requires --allow-read');
            }
        };
        global.fs = fs;
    "#)?;
    Ok(())
}