(function() {var implementors = {};
implementors["libloading"] = [];implementors["gfx_gl"] = [];implementors["shared_library"] = [];implementors["sdl2"] = [];implementors["tempfile"] = [];implementors["winit"] = [];implementors["glutin"] = [];

            if (window.register_implementors) {
                window.register_implementors(implementors);
            } else {
                window.pending_implementors = implementors;
            }
        
})()
